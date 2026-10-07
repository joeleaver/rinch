//! Review fixtures for PR #1429 (app image schemes + `reload_image`).
//!
//! Each test states what the PR's docs promise and what the code does. The
//! ones named `finding_*` FAIL at the PR head; the ones named `holds_*` pass
//! and pin a claimed behaviour the PR has no test for.

use rinch_core::NodeId;
use rinch_core::dom::DomDocument;
use rinch_core::image::{ImageLoadResult, ImageLoader, register_image_scheme};
use rinch_dom::RinchDocument;
use rinch_dom::image_cache::{has_pending, reload_image};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

const RED_3X2_PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x03, 0x00, 0x00, 0x00, 0x02, 0x08, 0x06, 0x00, 0x00, 0x00, 0x9d, 0x74, 0x66,
    0x1a, 0x00, 0x00, 0x00, 0x11, 0x49, 0x44, 0x41, 0x54, 0x78, 0xda, 0x63, 0xf8, 0xcf, 0xc0, 0xf0,
    0x1f, 0x86, 0x19, 0x90, 0x39, 0x00, 0x9b, 0x7e, 0x0b, 0xf5, 0x0f, 0x5f, 0x26, 0x22, 0x00, 0x00,
    0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

const ONE_BY_ONE_PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4,
    0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78, 0xda, 0x63, 0xf8, 0xcf, 0xc0, 0xf0,
    0x1f, 0x00, 0x05, 0x00, 0x01, 0xff, 0x56, 0xc7, 0x2f, 0x0d, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45,
    0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());
fn alone() -> std::sync::MutexGuard<'static, ()> {
    ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner())
}

struct Nothing;
impl ImageLoader for Nothing {
    fn load(&self, _src: &str) -> ImageLoadResult {
        ImageLoadResult::Failed("the default loader has nothing".into())
    }
}

fn doc_with_img(src: &str) -> (RinchDocument, NodeId) {
    let mut doc = RinchDocument::new();
    doc.tree.image_loader = Some(Arc::new(Nothing));
    let body = doc.body();
    let img = doc.create_element("img");
    doc.append_child(body, img);
    doc.set_attribute(img, "src", src);
    doc.resolve_layout(800.0, 600.0);
    (doc, img)
}

fn settle_for(
    doc: &mut RinchDocument,
    limit: Duration,
    mut done: impl FnMut(&RinchDocument) -> bool,
) -> bool {
    let deadline = Instant::now() + limit;
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

fn settle(doc: &mut RinchDocument, done: impl FnMut(&RinchDocument) -> bool) -> bool {
    settle_for(doc, Duration::from_secs(10), done)
}

fn box_of(doc: &RinchDocument, node: NodeId) -> (f32, f32) {
    let n = doc.tree.nodes.get(node.0).expect("node in tree");
    (n.layout.width, n.layout.height)
}

// ---------------------------------------------------------------------------
// F1. A loader that panics strands its source for the life of the document,
//     and `reload_image` cannot bring it back.
// ---------------------------------------------------------------------------

/// The app's loader panics once (a poisoned lock, an `unwrap` on a blob id it
/// cannot parse). The load thread dies before it pushes any answer, so the
/// cache entry stays `Loading`. `reload_image` on a `Loading` source only
/// *marks* it and waits for the answer in flight — which will never land.
#[test]
fn finding_a_loader_panic_strands_the_source_and_reload_cannot_recover_it() {
    let _alone = alone();
    let calls = Arc::new(AtomicUsize::new(0));
    let calls_in = calls.clone();
    register_image_scheme("review-panic", move |_src: &str| {
        if calls_in.fetch_add(1, Ordering::SeqCst) == 0 {
            panic!("the app's loader panicked (review fixture; expected on stderr)");
        }
        ImageLoadResult::Loaded(RED_3X2_PNG.to_vec())
    });
    let src = "review-panic:picture";
    let (mut doc, img) = doc_with_img(src);

    // The panic is an answer like any other failure: the source is failed,
    // not loading for ever.
    let failed = settle_for(&mut doc, Duration::from_secs(5), |d| {
        d.tree.image_cache.is_failed(src)
    });
    assert_eq!(calls.load(Ordering::SeqCst), 1, "control: called once");
    assert!(
        failed,
        "a loader panic must be recorded as a failed load, not leave the source loading"
    );
    assert!(!has_pending(doc.doc_key()), "and nothing is left queued");

    // The app asks again, once.
    reload_image(src);
    let came_back = settle_for(&mut doc, Duration::from_secs(5), |d| {
        d.tree.image_cache.get(src).is_some()
    });
    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "a reload must ask the loader again after its first call panicked"
    );
    assert!(came_back);
    assert_eq!(box_of(&doc, img), (3.0, 2.0));
}

/// A panic while a picture is on screen (its reload's loader call panics) is
/// a failed reload: the picture stays, and the next reload is a fresh one.
#[test]
fn a_loader_panic_during_the_reload_of_a_shown_picture_keeps_it_and_the_next_reload_works() {
    let _alone = alone();
    let calls = Arc::new(AtomicUsize::new(0));
    let calls_in = calls.clone();
    register_image_scheme("review-panic-shown", move |_src: &str| {
        match calls_in.fetch_add(1, Ordering::SeqCst) {
            0 => ImageLoadResult::Loaded(RED_3X2_PNG.to_vec()),
            1 => panic!("the app's loader panicked on a reload (review fixture; expected)"),
            _ => ImageLoadResult::Loaded(ONE_BY_ONE_PNG.to_vec()),
        }
    });
    let src = "review-panic-shown:picture";
    let (mut doc, img) = doc_with_img(src);
    assert!(settle(&mut doc, |d| d.tree.image_cache.get(src).is_some()));
    reload_image(src);
    assert!(settle(&mut doc, |d| calls.load(Ordering::SeqCst) == 2
        && !has_pending(d.doc_key())));
    settle_for(&mut doc, Duration::from_millis(100), |_| false);
    assert_eq!(box_of(&doc, img), (3.0, 2.0), "the picture it had stays");
    // The panicked reload is over: this one starts a load of its own (were it
    // still counted as in flight, the reload would wait for it for ever).
    reload_image(src);
    assert!(settle_for(&mut doc, Duration::from_secs(5), |d| box_of(
        d, img
    ) == (1.0, 1.0)));
    assert_eq!(calls.load(Ordering::SeqCst), 3);
}

// ---------------------------------------------------------------------------
// F2. Two reloads of a picture that is on screen race; the older answer can win.
// ---------------------------------------------------------------------------

/// A loader that reads the store at once and then, for the load numbered
/// `park`, waits at a gate before answering: a slow decode or a slow disk.
struct ParkOne {
    held: Arc<Mutex<HashMap<String, Vec<u8>>>>,
    calls: AtomicUsize,
    park: usize,
    gate: Arc<(Mutex<bool>, Condvar)>,
}

impl ImageLoader for ParkOne {
    fn load(&self, src: &str) -> ImageLoadResult {
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        let bytes = self.held.lock().unwrap().get(src).cloned();
        if n == self.park {
            let (lock, cv) = &*self.gate;
            let mut open = lock.lock().unwrap();
            while !*open {
                open = cv.wait(open).unwrap();
            }
        }
        match bytes {
            Some(b) => ImageLoadResult::Loaded(b),
            None => ImageLoadResult::Failed("not here".into()),
        }
    }
}

/// The PR protects a reload against an answer in flight only when the cache
/// entry is `Loading`. A *decoded* source that is reloaded stays `Decoded`
/// while its new load runs, so a second reload starts a second, unordered
/// load: whichever lands last is what the document shows. Here the app's last
/// word is the 3x2 picture and the document ends on the 1x1 one.
#[test]
fn finding_the_older_of_two_reloads_of_a_shown_picture_can_win() {
    let _alone = alone();
    let held = Arc::new(Mutex::new(HashMap::new()));
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let loader = Arc::new(ParkOne {
        held: held.clone(),
        calls: AtomicUsize::new(0),
        park: 1, // the first RELOAD (call 0 is the element's own load)
        gate: gate.clone(),
    });
    rinch_core::image::register_image_scheme_arc("review-race", loader.clone());
    let src = "review-race:picture";

    held.lock()
        .unwrap()
        .insert(src.to_string(), RED_3X2_PNG.to_vec());
    let (mut doc, img) = doc_with_img(src);
    assert!(settle(&mut doc, |d| d.tree.image_cache.get(src).is_some()));
    assert_eq!(box_of(&doc, img), (3.0, 2.0));

    // Edit 1: the picture becomes 1x1. Its load reads the 1x1 bytes and parks.
    held.lock()
        .unwrap()
        .insert(src.to_string(), ONE_BY_ONE_PNG.to_vec());
    reload_image(src);
    doc.resolve_layout(800.0, 600.0);
    let deadline = Instant::now() + Duration::from_secs(10);
    while loader.calls.load(Ordering::SeqCst) < 2 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(
        loader.calls.load(Ordering::SeqCst),
        2,
        "control: reload 1 is in flight"
    );
    std::thread::sleep(Duration::from_millis(50));

    // Edit 2: the picture becomes 3x2 again (an undo), while reload 1 is
    // still out. (At the PR head this started a second load at once, which
    // landed first; reload 1's older answer then replaced it.)
    held.lock()
        .unwrap()
        .insert(src.to_string(), RED_3X2_PNG.to_vec());
    reload_image(src);
    settle_for(&mut doc, Duration::from_millis(200), |_| false);
    assert_eq!(
        box_of(&doc, img),
        (3.0, 2.0),
        "control: nothing older has landed"
    );

    // Now reload 1's answer (older bytes) lands.
    {
        let (lock, cv) = &*gate;
        *lock.lock().unwrap() = true;
        cv.notify_all();
    }
    assert!(settle(&mut doc, |d| loader.calls.load(Ordering::SeqCst) == 3
        && !has_pending(d.doc_key())));
    settle_for(&mut doc, Duration::from_millis(300), |_| false);
    assert!(!has_pending(doc.doc_key()));
    assert_eq!(
        box_of(&doc, img),
        (3.0, 2.0),
        "the app's last reload named the 3x2 bytes; an answer asked for BEFORE it must not replace them"
    );
    assert_eq!(
        loader.calls.load(Ordering::SeqCst),
        3,
        "the element's load, reload 1, and one load for the reload asked for while it was out"
    );
}

/// The same race one state over: the source had FAILED (it is not on screen)
/// when its reload went out, and a second reload is asked for while the first
/// is still in flight. The first reload's answer was asked for before the
/// second, so it must not be what the document ends on.
#[test]
fn a_second_reload_of_a_failed_source_beats_the_first_reloads_older_answer() {
    let _alone = alone();
    let held = Arc::new(Mutex::new(HashMap::new()));
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let loader = Arc::new(ParkOne {
        held: held.clone(),
        calls: AtomicUsize::new(0),
        park: 1, // the first RELOAD
        gate: gate.clone(),
    });
    rinch_core::image::register_image_scheme_arc("review-failed-race", loader.clone());
    let src = "review-failed-race:picture";
    let (mut doc, img) = doc_with_img(src);
    assert!(settle(&mut doc, |d| d.tree.image_cache.is_failed(src)));

    // The blob arrives as a 1x1 picture: reload 1 reads it and parks.
    held.lock()
        .unwrap()
        .insert(src.to_string(), ONE_BY_ONE_PNG.to_vec());
    reload_image(src);
    assert!(settle(&mut doc, |_| loader.calls.load(Ordering::SeqCst) == 2));
    assert!(
        !doc.tree.image_cache.is_failed(src),
        "a failed source that is being reloaded is loading, not failed"
    );
    std::thread::sleep(Duration::from_millis(50));

    // It is replaced by a 3x2 one while reload 1 is out.
    held.lock()
        .unwrap()
        .insert(src.to_string(), RED_3X2_PNG.to_vec());
    reload_image(src);
    settle_for(&mut doc, Duration::from_millis(200), |_| false);
    {
        let (lock, cv) = &*gate;
        *lock.lock().unwrap() = true;
        cv.notify_all();
    }
    assert!(settle(&mut doc, |d| loader.calls.load(Ordering::SeqCst) == 3
        && !has_pending(d.doc_key())));
    settle_for(&mut doc, Duration::from_millis(300), |_| false);
    assert_eq!(box_of(&doc, img), (3.0, 2.0));
    assert_eq!(loader.calls.load(Ordering::SeqCst), 3);
}

/// Reloads asked for before the document's next layout are one reload: the
/// loader is asked once, not once per call.
#[test]
fn reloads_queued_before_a_layout_are_one_reload() {
    let _alone = alone();
    let held: Arc<Mutex<Option<Vec<u8>>>> = Arc::new(Mutex::new(None));
    let held_in = held.clone();
    let asked = Arc::new(AtomicUsize::new(0));
    let asked_in = asked.clone();
    register_image_scheme("review-queued", move |_src: &str| {
        asked_in.fetch_add(1, Ordering::SeqCst);
        match held_in.lock().unwrap().clone() {
            Some(b) => ImageLoadResult::Loaded(b),
            None => ImageLoadResult::Failed("not yet".into()),
        }
    });
    let src = "review-queued:picture";
    let (mut doc, img) = doc_with_img(src);
    assert!(settle(&mut doc, |d| d.tree.image_cache.is_failed(src)));
    assert_eq!(asked.load(Ordering::SeqCst), 1);
    *held.lock().unwrap() = Some(RED_3X2_PNG.to_vec());
    for _ in 0..5 {
        reload_image(src);
    }
    assert!(settle(&mut doc, |d| d.tree.image_cache.get(src).is_some()));
    settle_for(&mut doc, Duration::from_millis(300), |_| false);
    assert_eq!(box_of(&doc, img), (3.0, 2.0));
    assert_eq!(
        asked.load(Ordering::SeqCst),
        2,
        "five reload_image calls before one layout ask the loader once"
    );
}

/// Every `RinchDocument::reload_image` of a decoded source starts a thread and
/// a loader call: there is no "a reload of this source is already running".
#[test]
fn finding_reloads_of_a_shown_picture_are_not_deduplicated() {
    let _alone = alone();
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let held = Arc::new(Mutex::new(HashMap::new()));
    let loader = Arc::new(ParkOne {
        held: held.clone(),
        calls: AtomicUsize::new(0),
        // The first reload's load is slow, so the other seven are asked for
        // while it is out. (The review's version let every load finish at
        // once, which made the count depend on whether a load thread or a
        // layout was quicker; a reload asked for after the last load FINISHED
        // does need a load of its own.)
        park: 1,
        gate: gate.clone(),
    });
    rinch_core::image::register_image_scheme_arc("review-dedup", loader.clone());
    let src = "review-dedup:picture";
    held.lock()
        .unwrap()
        .insert(src.to_string(), RED_3X2_PNG.to_vec());
    let (mut doc, img) = doc_with_img(src);
    assert!(settle(&mut doc, |d| d.tree.image_cache.get(src).is_some()));
    assert_eq!(loader.calls.load(Ordering::SeqCst), 1);

    // Eight reloads, a layout between each (a sync loop reporting progress).
    for i in 0..8 {
        if i == 7 {
            held.lock()
                .unwrap()
                .insert(src.to_string(), ONE_BY_ONE_PNG.to_vec());
        }
        reload_image(src);
        doc.resolve_layout(800.0, 600.0);
    }
    settle_for(&mut doc, Duration::from_millis(200), |_| false);
    assert_eq!(
        loader.calls.load(Ordering::SeqCst) - 1,
        1,
        "while one reload of a shown source is out, further reloads start no load of their own"
    );
    {
        let (lock, cv) = &*gate;
        *lock.lock().unwrap() = true;
        cv.notify_all();
    }
    assert!(settle(&mut doc, |d| box_of(d, img) == (1.0, 1.0)));
    settle_for(&mut doc, Duration::from_millis(300), |_| false);
    assert_eq!(
        loader.calls.load(Ordering::SeqCst) - 1,
        2,
        "8 reloads of one shown source: the one in flight plus ONE behind it, which reads what \
         the last of them named"
    );
    assert_eq!(box_of(&doc, img), (1.0, 1.0));
}

// ---------------------------------------------------------------------------
// F3. "`data:` is never offered to a loader" — a background `data:` URL is.
// ---------------------------------------------------------------------------

#[test]
fn finding_a_registered_data_scheme_is_offered_a_background_data_url() {
    let _alone = alone();
    let asked = Arc::new(AtomicUsize::new(0));
    let asked_in = asked.clone();
    // At the PR head the registration was accepted without complaint and the
    // loader was then asked twice. `data` is refused like any other scheme no
    // app loader can answer for (the panic's message is expected on stderr).
    let refused = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        register_image_scheme("Data", move |_src: &str| {
            asked_in.fetch_add(1, Ordering::SeqCst);
            ImageLoadResult::Failed("the app's `data` loader".into())
        })
    }))
    .is_err();
    let mut doc = RinchDocument::new();
    doc.tree.image_loader = Some(Arc::new(Nothing));
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(
        div,
        "style",
        "width: 10px; height: 10px; background-image: url(data:image/png;base64,AAAA)",
    );
    doc.append_child(body, div);
    let img = doc.create_element("img");
    doc.append_child(body, img);
    doc.set_attribute(img, "src", "data:image/png;base64,BBBB");
    settle_for(&mut doc, Duration::from_millis(300), |_| false);
    let n = asked.load(Ordering::SeqCst);
    rinch_core::image::unregister_image_scheme("data");
    assert!(refused, "registering the `data` scheme must panic");
    assert_eq!(n, 0, "no app loader is ever offered a `data:` URL");
}

// ---------------------------------------------------------------------------
// Claimed behaviours the PR has no test for.
// ---------------------------------------------------------------------------

/// "every `<img>` and `background-image` naming `src` ... picks the picture
/// up": the background half, and that its user is repainted.
#[test]
fn holds_a_failed_background_image_is_reloaded_and_its_user_repainted() {
    let _alone = alone();
    let held: Arc<Mutex<Option<Vec<u8>>>> = Arc::new(Mutex::new(None));
    let held_in = held.clone();
    let asked = Arc::new(AtomicUsize::new(0));
    let asked_in = asked.clone();
    register_image_scheme("review-bg", move |_src: &str| {
        asked_in.fetch_add(1, Ordering::SeqCst);
        match held_in.lock().unwrap().clone() {
            Some(b) => ImageLoadResult::Loaded(b),
            None => ImageLoadResult::Failed("not yet".into()),
        }
    });
    let src = "review-bg:tile";
    let mut doc = RinchDocument::new();
    doc.tree.image_loader = Some(Arc::new(Nothing));
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(
        div,
        "style",
        "width: 10px; height: 10px; background-image: url(review-bg:tile)",
    );
    doc.append_child(body, div);
    assert!(settle(&mut doc, |d| d.tree.image_cache.is_failed(src)));
    assert_eq!(asked.load(Ordering::SeqCst), 1);
    // Idle: consume what the first layouts dirtied.
    doc.tree.consume_paint_dirty();
    doc.resolve_layout(800.0, 600.0);
    doc.tree.consume_paint_dirty();
    assert!(doc.tree.paint_dirty_nodes.is_empty(), "control: idle");

    *held.lock().unwrap() = Some(RED_3X2_PNG.to_vec());
    reload_image(src);
    let mut dirtied = false;
    assert!(settle(&mut doc, |d| {
        dirtied |= d.tree.paint_dirty_nodes.contains(&div.0);
        d.tree.image_cache.get(src).is_some()
    }));
    assert_eq!(asked.load(Ordering::SeqCst), 2);
    assert!(
        dirtied,
        "the element drawing the background is in the damage"
    );
}

/// A reload reaches every live document that knows the source (#134).
#[test]
fn holds_a_reload_reaches_two_documents_and_each_asks_once() {
    let _alone = alone();
    let held: Arc<Mutex<Option<Vec<u8>>>> = Arc::new(Mutex::new(None));
    let held_in = held.clone();
    let asked = Arc::new(AtomicUsize::new(0));
    let asked_in = asked.clone();
    register_image_scheme("review-two", move |_src: &str| {
        asked_in.fetch_add(1, Ordering::SeqCst);
        match held_in.lock().unwrap().clone() {
            Some(b) => ImageLoadResult::Loaded(b),
            None => ImageLoadResult::Failed("not yet".into()),
        }
    });
    let src = "review-two:picture";
    let (mut a, img_a) = doc_with_img(src);
    let (mut b, img_b) = doc_with_img(src);
    assert!(settle(&mut a, |d| d.tree.image_cache.is_failed(src)));
    assert!(settle(&mut b, |d| d.tree.image_cache.is_failed(src)));
    assert_eq!(asked.load(Ordering::SeqCst), 2);
    *held.lock().unwrap() = Some(RED_3X2_PNG.to_vec());
    reload_image(src);
    assert!(has_pending(a.doc_key()) && has_pending(b.doc_key()));
    assert!(settle(&mut a, |d| d.tree.image_cache.get(src).is_some()));
    assert!(settle(&mut b, |d| d.tree.image_cache.get(src).is_some()));
    assert_eq!(box_of(&a, img_a), (3.0, 2.0));
    assert_eq!(box_of(&b, img_b), (3.0, 2.0));
    assert_eq!(asked.load(Ordering::SeqCst), 4);
}

/// The answer had already LANDED in the queue (not yet drained) when the app
/// reloaded: it predates the reload and must not be what the cache keeps.
/// Kills the mutant that drains answers before it applies queued reloads.
#[test]
fn holds_a_reload_beats_an_answer_that_landed_but_was_not_drained() {
    let _alone = alone();
    let held: Arc<Mutex<Option<Vec<u8>>>> = Arc::new(Mutex::new(None));
    let held_in = held.clone();
    let asked = Arc::new(AtomicUsize::new(0));
    let asked_in = asked.clone();
    register_image_scheme("review-landed", move |_src: &str| {
        asked_in.fetch_add(1, Ordering::SeqCst);
        match held_in.lock().unwrap().clone() {
            Some(b) => ImageLoadResult::Loaded(b),
            None => ImageLoadResult::Failed("not yet".into()),
        }
    });
    let src = "review-landed:picture";
    // No layout after the `src` is set: a layout would drain the miss.
    let mut doc = RinchDocument::new();
    doc.tree.image_loader = Some(Arc::new(Nothing));
    let body = doc.body();
    let img = doc.create_element("img");
    doc.append_child(body, img);
    doc.resolve_layout(800.0, 600.0);
    doc.set_attribute(img, "src", src);
    // The miss lands in the queue; no layout runs, so it is not drained.
    let deadline = Instant::now() + Duration::from_secs(10);
    while !has_pending(doc.doc_key()) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(has_pending(doc.doc_key()), "control: the miss is queued");
    assert!(!doc.tree.image_cache.is_failed(src), "control: undrained");
    *held.lock().unwrap() = Some(RED_3X2_PNG.to_vec());
    reload_image(src);
    assert!(settle(&mut doc, |d| d.tree.image_cache.get(src).is_some()));
    assert_eq!(box_of(&doc, img), (3.0, 2.0));
    assert_eq!(asked.load(Ordering::SeqCst), 2);
}

/// Scheme spelling: the registry is case-insensitive, the cache key is not.
/// `reload_image` must be given the source exactly as the element spelled it.
#[test]
fn holds_reload_is_keyed_by_the_source_as_spelled() {
    let _alone = alone();
    let held: Arc<Mutex<Option<Vec<u8>>>> = Arc::new(Mutex::new(None));
    let held_in = held.clone();
    register_image_scheme("review-case", move |_src: &str| {
        match held_in.lock().unwrap().clone() {
            Some(b) => ImageLoadResult::Loaded(b),
            None => ImageLoadResult::Failed("not yet".into()),
        }
    });
    let (mut doc, img) = doc_with_img("Review-Case:picture");
    assert!(settle(&mut doc, |d| d
        .tree
        .image_cache
        .is_failed("Review-Case:picture")));
    *held.lock().unwrap() = Some(RED_3X2_PNG.to_vec());
    reload_image("review-case:picture"); // lowercased scheme: another source
    settle_for(&mut doc, Duration::from_millis(200), |_| false);
    assert_eq!(
        box_of(&doc, img),
        (0.0, 0.0),
        "a different spelling is a different source"
    );
    reload_image("Review-Case:picture");
    assert!(settle(&mut doc, |d| d
        .tree
        .image_cache
        .get("Review-Case:picture")
        .is_some()));
    assert_eq!(box_of(&doc, img), (3.0, 2.0));
}
