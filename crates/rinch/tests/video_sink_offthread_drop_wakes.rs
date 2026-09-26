//! Issue #1035: a video frame sink dropped off its surface's thread queues its
//! surface's release for the main thread — and that release must be queued the
//! way every other cross-thread closure is, through the host's dispatcher, so
//! the host is woken for it.
//!
//! A host that wakes coalesces its wakes on the main-thread queue: it wakes
//! only for a push onto an *empty* queue (`queue_main_callback` answering
//! `true`), because a non-empty queue already has a wake owed. A release pushed
//! behind the dispatcher's back breaks that invariant twice over: it is queued
//! with no wake of its own, and the next sender — any `run_on_main_thread` or
//! `Signal::send`, from any thread — finds the queue non-empty, asks for no
//! wake either, and waits behind it for an unrelated event.
//!
//! The dispatcher here is the desktop shell's rule (`dispatch_to_main_thread`
//! in `shell/rinch_runtime.rs`: queue, and wake when the queue was empty),
//! counting its wakes instead of posting a winit `ReRender`. Its own binary,
//! because it registers the process-global dispatcher and main thread.
#![cfg(feature = "desktop")]

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use rinch::render_surface::{create_video_frame_sink, registered_viewport_names};

static WAKES: AtomicUsize = AtomicUsize::new(0);
/// The queue, the dispatcher and the wake count are process-global.
static SERIAL: Mutex<()> = Mutex::new(());

fn waking_dispatcher(f: Box<dyn FnOnce() + Send>) {
    if rinch_core::queue_main_callback(f) {
        WAKES.fetch_add(1, Ordering::SeqCst);
    }
}

fn surfaces_named(name: &str) -> usize {
    registered_viewport_names()
        .iter()
        .filter(|n| n.as_str() == name)
        .count()
}

/// Registers the dispatcher, proves it wakes, and leaves the queue empty and
/// the wake count at zero.
fn setup() -> std::sync::MutexGuard<'static, ()> {
    let guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    rinch_core::register_main_thread();
    rinch_core::set_cross_thread_dispatcher(waking_dispatcher);
    rinch_core::drain_main_callbacks();
    WAKES.store(0, Ordering::SeqCst);

    // Positive control: the dispatcher is live, and wakes for a closure queued
    // from another thread onto an empty queue.
    std::thread::spawn(|| rinch_core::run_on_main_thread(|| {}))
        .join()
        .unwrap();
    assert_eq!(WAKES.load(Ordering::SeqCst), 1, "the dispatcher woke once");
    rinch_core::drain_main_callbacks();
    WAKES.store(0, Ordering::SeqCst);
    guard
}

/// The release alone is owed a wake: nothing else is queued, so without one an
/// idle host never runs it.
#[test]
fn an_off_thread_sink_drop_wakes_the_host() {
    let _serial = setup();
    let name = "video-1035-first";
    let sink = create_video_frame_sink(name);
    std::thread::spawn(move || drop(sink)).join().unwrap();
    assert!(
        rinch_core::main_callbacks_pending(),
        "the release is queued"
    );
    assert_eq!(
        WAKES.load(Ordering::SeqCst),
        1,
        "a sink dropped off-thread queued its release with no wake"
    );
    rinch_core::drain_main_callbacks();
    assert_eq!(surfaces_named(name), 0, "the release ran on the drain");
}

/// The release must not swallow the wake of whoever queues next.
#[test]
fn an_off_thread_sink_drop_keeps_the_next_senders_wake() {
    let _serial = setup();
    // Off the fixed point: the second sender is an unrelated closure from
    // another thread, queued behind the release.
    let name = "video-1035-second";
    let sink = create_video_frame_sink(name);
    std::thread::spawn(move || drop(sink)).join().unwrap();
    let ran = Arc::new(AtomicBool::new(false));
    let flag = ran.clone();
    std::thread::spawn(move || {
        rinch_core::run_on_main_thread(move || flag.store(true, Ordering::SeqCst))
    })
    .join()
    .unwrap();
    assert!(
        WAKES.load(Ordering::SeqCst) >= 1,
        "the release and the closure queued behind it are both stranded: \
         queued, and no wake owed for either"
    );
    rinch_core::drain_main_callbacks();
    assert!(ran.load(Ordering::SeqCst), "the closure ran on the drain");
    assert_eq!(surfaces_named(name), 0, "the release ran on the drain");
}
