//! A registered name asked for by documents on one thread while another
//! thread registers and unregisters it (#2857, review of #2860): the two never
//! wait on each other for good, and once both have finished the document
//! shows what the registry holds last.
//!
//! What this guards is the **lock order**. A document resolves a
//! `rinch-image:` source holding the shared store's lock while it reads the
//! registry and the app's loaders (`resolve_registered`); `register_rgba` and
//! `unregister_rgba` let go of the registry's lock before they call into the
//! store (`reload_image` → `forget_shared`). Store, then registry; never the
//! reverse. A path that held the registry's lock across its call into the
//! store would deadlock against a resolve, and these rounds hang on it: each
//! test runs them on a thread of its own and fails when no round has finished
//! in [`LIMIT`].
//!
//! The race the review found (a document that read the old pixels and was
//! missed by the change's reload) is **not reliably** caught here: its window
//! is short. With either half of the fix reverted, 3,000 rounds ended stale
//! in none on a quiet machine, and in 2 to 8 on one at a load of 50 to 130,
//! where threads are preempted inside the window. The race is closed by the
//! order of the steps, which `resolve_registered` argues. A right
//! implementation never ends a round stale: none in 3,000 at a load of
//! about 80, and none in 3,000 at about 70.
//!
//! Neither limit times work that load can stretch to it: a round takes
//! milliseconds (about 70 at a load of 80), and only a stale round waits
//! [`SETTLE`] out.

#![cfg(feature = "software-renderer")]

use rinch_core::dom::DomDocument;
use rinch_core::image::{register_rgba, rgba_src, unregister_rgba};
use rinch_dom::RinchDocument;
use rinch_dom::image_cache::FileImageLoader;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Barrier, Mutex, MutexGuard, mpsc};
use std::time::{Duration, Instant};

/// The rounds of each test.
const ROUNDS: u32 = 300;

/// How long a round may take: one takes milliseconds, a stale one at most
/// [`SETTLE`]; a deadlocked one never finishes.
const LIMIT: Duration = Duration::from_secs(60);

/// How long a document is laid out, waiting for what the registry holds
/// last, before its round counts as stale: a right one gets there in a frame
/// or two, once the reload's answer lands.
const SETTLE: Duration = Duration::from_secs(20);

/// The tests one at a time: each spins a registering thread.
static ALONE: Mutex<()> = Mutex::new(());

fn alone() -> MutexGuard<'static, ()> {
    ALONE.lock().unwrap_or_else(|e| e.into_inner())
}

/// A 1x1 picture whose red channel is `red`.
fn pixel(red: u8) -> Arc<[u8]> {
    vec![red, 0, 0, 255].into()
}

/// A document with one `<img>` naming `src`: the request is made as the
/// attribute is set.
fn naming(src: &str) -> RinchDocument {
    let mut doc = RinchDocument::new();
    doc.tree.image_loader = Some(Arc::new(FileImageLoader));
    let body = doc.body();
    let img = doc.create_element("img");
    doc.set_attribute(img, "src", src);
    doc.append_child(body, img);
    doc
}

/// Lays `doc` out until `done` holds, or [`SETTLE`] passes: what holds then.
fn settles(doc: &mut RinchDocument, done: impl Fn(&RinchDocument) -> bool) -> bool {
    let deadline = Instant::now() + SETTLE;
    let mut wide = false;
    loop {
        wide = !wide;
        doc.resolve_layout(if wide { 41.0 } else { 40.0 }, 40.0);
        if done(doc) {
            return true;
        }
        if Instant::now() > deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// What the rounds' thread reports.
enum Report {
    /// A round finished.
    Round,
    /// All of them did: how many ended stale.
    Done(u32),
}

/// `ROUNDS` rounds: a new document names `name` while another thread runs
/// `change` again and again, its last call leaving the registry as
/// `last_is_registered` says; then the document settles on that. Each round
/// is reported to `report` as it finishes; returns how many ended otherwise.
fn rounds(
    name: &str,
    change: fn(&str, u8),
    last_is_registered: bool,
    report: &mpsc::Sender<Report>,
) -> u32 {
    let src = rgba_src(name);
    let mut stale = 0;
    for round in 0..ROUNDS {
        register_rgba(name, 1, 1, pixel(1)).unwrap();
        let go = Arc::new(Barrier::new(2));
        let stop = Arc::new(AtomicBool::new(false));
        let changer = {
            let (go, stop, name) = (go.clone(), stop.clone(), name.to_owned());
            std::thread::spawn(move || {
                go.wait();
                let mut n: u8 = 2;
                while !stop.load(Ordering::Relaxed) {
                    change(&name, n);
                    n = n.wrapping_add(1).max(2);
                }
                // The last word.
                if last_is_registered {
                    register_rgba(&name, 1, 1, pixel(255)).unwrap();
                } else {
                    unregister_rgba(&name);
                }
            })
        };
        go.wait();
        let mut doc = naming(&src);
        stop.store(true, Ordering::Relaxed);
        changer.join().unwrap();
        let right = settles(&mut doc, |doc| {
            if last_is_registered {
                doc.tree
                    .image_cache
                    .get(&src)
                    .is_some_and(|img| img.premultiplied()[0] == 255)
            } else {
                doc.tree.image_cache.shows_failure(&src)
            }
        });
        if !right {
            stale += 1;
            eprintln!("round {round}: the document kept what it read before the last change");
        }
        report.send(Report::Round).ok();
    }
    unregister_rgba(name);
    stale
}

/// [`rounds`] on a thread of its own, failing when no round has finished in
/// [`LIMIT`] (the threads a deadlock holds are left to the process's end).
fn rounds_within_the_limit(name: &'static str, change: fn(&str, u8), last_is_registered: bool) {
    let (report, reports) = mpsc::channel();
    std::thread::spawn(move || {
        let stale = rounds(name, change, last_is_registered, &report);
        report.send(Report::Done(stale)).ok();
    });
    let mut finished = 0;
    loop {
        match reports.recv_timeout(LIMIT) {
            Ok(Report::Round) => finished += 1,
            Ok(Report::Done(stale)) => {
                assert_eq!(stale, 0, "{stale} of {ROUNDS} rounds ended stale");
                return;
            }
            Err(_) => panic!(
                "no round of resolving and registering finished in {LIMIT:?} (after {finished} \
                 of {ROUNDS}): a lock taken in the other order (store, then registry; never the \
                 reverse)"
            ),
        }
    }
}

fn register_again(name: &str, n: u8) {
    register_rgba(name, 1, 1, pixel(n)).unwrap();
}

fn unregister_and_register(name: &str, n: u8) {
    if n.is_multiple_of(2) {
        unregister_rgba(name);
    } else {
        register_rgba(name, 1, 1, pixel(n)).unwrap();
    }
}

#[test]
fn resolving_while_another_thread_registers_never_waits_for_good() {
    let _alone = alone();
    rounds_within_the_limit("t2860-registered", register_again, true);
}

#[test]
fn resolving_while_another_thread_unregisters_never_waits_for_good() {
    let _alone = alone();
    rounds_within_the_limit("t2860-unregistered", unregister_and_register, false);
}
