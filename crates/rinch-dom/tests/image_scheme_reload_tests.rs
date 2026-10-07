//! An app answers for its own image URL schemes, and a source that failed (or
//! changed) can be asked for again.
//!
//! Two things an app that keeps its pictures somewhere only it can reach
//! needs, neither of which the loader a document is built with gave it:
//!
//! - `register_image_scheme`: sources with that scheme go to the app's loader,
//!   every other source still goes to the document's own.
//! - `reload_image`: a load's answer is cached by source, a failure included,
//!   so a picture whose bytes arrive after it was first asked for never showed
//!   without this. It also has to win against an answer that was already on
//!   its way, and must not blank a picture that is on screen.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_core::image::{ImageLoadResult, ImageLoader, register_image_scheme};
use rinch_dom::RinchDocument;
use rinch_dom::image_cache::{has_pending, reload_image};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// A 3x2 opaque-red PNG.
const RED_3X2_PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x03, 0x00, 0x00, 0x00, 0x02, 0x08, 0x06, 0x00, 0x00, 0x00, 0x9d, 0x74, 0x66,
    0x1a, 0x00, 0x00, 0x00, 0x11, 0x49, 0x44, 0x41, 0x54, 0x78, 0xda, 0x63, 0xf8, 0xcf, 0xc0, 0xf0,
    0x1f, 0x86, 0x19, 0x90, 0x39, 0x00, 0x9b, 0x7e, 0x0b, 0xf5, 0x0f, 0x5f, 0x26, 0x22, 0x00, 0x00,
    0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

/// A 1x1 PNG, so a reload that lands is told from the picture it replaces by
/// its size.
const ONE_BY_ONE_PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4,
    0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78, 0xda, 0x63, 0xf8, 0xcf, 0xc0, 0xf0,
    0x1f, 0x00, 0x05, 0x00, 0x01, 0xff, 0x56, 0xc7, 0x2f, 0x0d, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45,
    0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

/// `reload_image` reaches every live document in the process, so a reload in
/// one test is (harmlessly) queued for the documents of the others. The tests
/// that assert a document is idle therefore run one at a time.
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

fn alone() -> std::sync::MutexGuard<'static, ()> {
    ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner())
}

/// A store of bytes by source, as an app's blob store is: a source it does not
/// hold is a failure, and it counts how often it was asked.
#[derive(Default)]
struct Blobs {
    held: Mutex<std::collections::HashMap<String, Vec<u8>>>,
    asked: AtomicUsize,
}

impl Blobs {
    fn put(&self, src: &str, bytes: &[u8]) {
        self.held.lock().unwrap().insert(src.into(), bytes.to_vec());
    }
    fn asked(&self) -> usize {
        self.asked.load(Ordering::SeqCst)
    }
}

struct BlobLoader(Arc<Blobs>);

impl ImageLoader for BlobLoader {
    fn load(&self, src: &str) -> ImageLoadResult {
        self.0.asked.fetch_add(1, Ordering::SeqCst);
        match self.0.held.lock().unwrap().get(src) {
            Some(bytes) => ImageLoadResult::Loaded(bytes.clone()),
            None => ImageLoadResult::Failed(format!("{src} is not here yet")),
        }
    }
}

/// The document's own loader in these tests: it fails everything and counts,
/// so a source that reached it instead of the app's scheme loader shows.
struct DefaultLoader(Arc<AtomicUsize>);

impl ImageLoader for DefaultLoader {
    fn load(&self, _src: &str) -> ImageLoadResult {
        self.0.fetch_add(1, Ordering::SeqCst);
        ImageLoadResult::Failed("the default loader has nothing".into())
    }
}

fn doc_with_img(src: &str) -> (RinchDocument, NodeId, Arc<AtomicUsize>) {
    let mut doc = RinchDocument::new();
    let default_loads = Arc::new(AtomicUsize::new(0));
    doc.tree.image_loader = Some(Arc::new(DefaultLoader(default_loads.clone())));
    let body = doc.body();
    let img = doc.create_element("img");
    doc.append_child(body, img);
    doc.set_attribute(img, "src", src);
    doc.resolve_layout(800.0, 600.0);
    (doc, img, default_loads)
}

/// Lay out until `done` holds (the loads are on background threads).
fn settle(doc: &mut RinchDocument, mut done: impl FnMut(&RinchDocument) -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(10);
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

fn box_of(doc: &RinchDocument, node: NodeId) -> (f32, f32) {
    let n = doc.tree.nodes.get(node.0).expect("node in tree");
    (n.layout.width, n.layout.height)
}

#[test]
fn a_registered_scheme_is_loaded_by_the_apps_loader_and_other_sources_by_the_documents() {
    let _alone = alone();
    let blobs = Arc::new(Blobs::default());
    blobs.put("scheme-test-own:red", RED_3X2_PNG);
    register_image_scheme("scheme-test-own", BlobLoader(blobs.clone()));

    let (mut doc, img, default_loads) = doc_with_img("scheme-test-own:red");
    assert!(
        settle(&mut doc, |d| d
            .tree
            .image_cache
            .get("scheme-test-own:red")
            .is_some()),
        "the app's loader answered, so the picture decodes"
    );
    assert_eq!(box_of(&doc, img), (3.0, 2.0));
    assert_eq!(blobs.asked(), 1);
    assert_eq!(
        default_loads.load(Ordering::SeqCst),
        0,
        "the document's loader is not asked for a scheme the app answers"
    );

    // A source with no registered scheme still goes to the document's loader.
    let body = doc.body();
    let other = doc.create_element("img");
    doc.append_child(body, other);
    doc.set_attribute(other, "src", "pictures/elsewhere.png");
    assert!(settle(&mut doc, |d| d
        .tree
        .image_cache
        .is_failed("pictures/elsewhere.png")));
    assert_eq!(default_loads.load(Ordering::SeqCst), 1);
    assert_eq!(blobs.asked(), 1, "and the app's loader is not asked for it");
}

#[test]
fn a_failed_source_is_not_asked_for_again_until_it_is_reloaded() {
    let _alone = alone();
    let blobs = Arc::new(Blobs::default());
    register_image_scheme("scheme-test-late", BlobLoader(blobs.clone()));
    let src = "scheme-test-late:picture";

    let (mut doc, img, _) = doc_with_img(src);
    assert!(settle(&mut doc, |d| d.tree.image_cache.is_failed(src)));
    assert_eq!(box_of(&doc, img), (0.0, 0.0));
    assert_eq!(blobs.asked(), 1);

    // The failure is cached: a second element naming the source, and any
    // number of layouts, ask nothing.
    let body = doc.body();
    let second = doc.create_element("img");
    doc.append_child(body, second);
    doc.set_attribute(second, "src", src);
    for _ in 0..5 {
        doc.resolve_layout(800.0, 600.0);
    }
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(blobs.asked(), 1, "a miss is remembered");

    // The bytes arrive. Nothing notices on its own...
    blobs.put(src, RED_3X2_PNG);
    doc.resolve_layout(800.0, 600.0);
    assert!(doc.tree.image_cache.is_failed(src));
    assert!(!has_pending(doc.doc_key()), "and the document is idle");

    // ...until the app says so, from any thread.
    let from_elsewhere = std::thread::spawn(move || reload_image(src));
    from_elsewhere.join().unwrap();
    assert!(
        has_pending(doc.doc_key()),
        "a reload is a reason to lay out: every host's frame gate asks this"
    );
    assert!(settle(&mut doc, |d| d.tree.image_cache.get(src).is_some()));
    assert_eq!(blobs.asked(), 2);
    assert_eq!(box_of(&doc, img), (3.0, 2.0), "the first element shows it");
    assert_eq!(box_of(&doc, second), (3.0, 2.0), "and so does the second");
    assert!(!has_pending(doc.doc_key()), "one reload, not one per frame");
}

/// A loader whose first call parks until released and then says "not here":
/// the answer that was already on its way when the bytes arrived.
struct StaleFirstAnswer {
    blobs: Arc<Blobs>,
    gate: Arc<(Mutex<bool>, Condvar)>,
    calls: AtomicUsize,
}

impl ImageLoader for StaleFirstAnswer {
    fn load(&self, src: &str) -> ImageLoadResult {
        if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
            let (lock, cv) = &*self.gate;
            let mut open = lock.lock().unwrap();
            while !*open {
                open = cv.wait(open).unwrap();
            }
            return ImageLoadResult::Failed("asked before the bytes arrived".into());
        }
        BlobLoader(self.blobs.clone()).load(src)
    }
}

#[test]
fn a_reload_wins_against_an_answer_that_was_already_in_flight() {
    let _alone = alone();
    let blobs = Arc::new(Blobs::default());
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    register_image_scheme(
        "scheme-test-race",
        StaleFirstAnswer {
            blobs: blobs.clone(),
            gate: gate.clone(),
            calls: AtomicUsize::new(0),
        },
    );
    let src = "scheme-test-race:picture";

    // The first load is in flight (parked) when the bytes arrive and the app
    // reloads.
    let (mut doc, img, _) = doc_with_img(src);
    blobs.put(src, RED_3X2_PNG);
    reload_image(src);
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(
        blobs.asked(),
        0,
        "nothing new is started beside a load in flight"
    );

    // Now the stale "not here" lands. It must not be what the cache keeps.
    {
        let (lock, cv) = &*gate;
        *lock.lock().unwrap() = true;
        cv.notify_all();
    }
    assert!(
        settle(&mut doc, |d| d.tree.image_cache.get(src).is_some()),
        "the stale failure is dropped and the source asked for once more"
    );
    assert_eq!(blobs.asked(), 1);
    assert_eq!(box_of(&doc, img), (3.0, 2.0));
}

#[test]
fn reloading_a_picture_that_is_on_screen_keeps_it_until_the_new_one_lands() {
    let _alone = alone();
    let blobs = Arc::new(Blobs::default());
    register_image_scheme("scheme-test-swap", BlobLoader(blobs.clone()));
    let src = "scheme-test-swap:picture";
    blobs.put(src, RED_3X2_PNG);

    let (mut doc, img, _) = doc_with_img(src);
    assert!(settle(&mut doc, |d| d.tree.image_cache.get(src).is_some()));
    assert_eq!(box_of(&doc, img), (3.0, 2.0));

    // New bytes under the same source.
    blobs.put(src, ONE_BY_ONE_PNG);
    doc.reload_image(src);
    assert!(
        doc.tree.image_cache.get(src).is_some(),
        "the old pixels stay in the cache while the new ones load"
    );
    assert!(settle(&mut doc, |d| {
        d.tree.image_cache.get(src).is_some_and(|i| i.width == 1)
    }));
    assert_eq!(box_of(&doc, img), (1.0, 1.0), "and the box follows");

    // A reload that fails leaves the picture it had.
    blobs.held.lock().unwrap().remove(src);
    let asked = blobs.asked();
    doc.reload_image(src);
    assert!(settle(&mut doc, |d| {
        blobs.asked() > asked && !has_pending(d.doc_key())
    }));
    std::thread::sleep(Duration::from_millis(50));
    doc.resolve_layout(800.0, 600.0);
    assert!(doc.tree.image_cache.get(src).is_some());
    assert_eq!(box_of(&doc, img), (1.0, 1.0));
}

#[test]
fn reloading_a_source_nothing_asked_for_starts_nothing() {
    let _alone = alone();
    let blobs = Arc::new(Blobs::default());
    register_image_scheme("scheme-test-unknown", BlobLoader(blobs.clone()));
    let (mut doc, _, _) = doc_with_img("scheme-test-unknown:shown");
    assert!(settle(&mut doc, |d| d
        .tree
        .image_cache
        .is_failed("scheme-test-unknown:shown")));

    reload_image("scheme-test-unknown:never-named");
    reload_image("data:image/png;base64,AAAA");
    for _ in 0..5 {
        doc.resolve_layout(800.0, 600.0);
    }
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(blobs.asked(), 1, "only the element's own first load");
    assert!(
        !doc.tree
            .image_cache
            .contains("scheme-test-unknown:never-named")
    );
    assert!(!has_pending(doc.doc_key()));
}

#[test]
fn a_dropped_document_leaves_no_reload_queued() {
    let _alone = alone();
    let (doc, _, _) = doc_with_img("scheme-test-dropped.png");
    let key = doc.doc_key();
    reload_image("scheme-test-dropped.png");
    assert!(has_pending(key));
    drop(doc);
    assert!(!has_pending(key));
    // And a reload after the drop queues nothing for it.
    reload_image("scheme-test-dropped.png");
    assert!(!has_pending(key));
}
