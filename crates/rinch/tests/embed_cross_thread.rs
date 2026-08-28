//! Empirical pinning tests for issue #172: a background thread can write a
//! signal in an embedded `RinchContext`.
//!
//! `RinchContext::new` calls `rinch_core::register_main_thread()`, and that call
//! is what *arms* the cross-thread check — after it, `is_main_thread()` stops
//! answering `true` unconditionally. It used to be the only half embed
//! registered: with `CROSS_THREAD_DISPATCHER` still `None`, a background-thread
//! `Signal::send()` / `update_send()` / `run_on_main_thread()` reached
//! `dispatch_to_main_thread` and **panicked on the calling thread**. Arming the
//! check without arming the transport is strictly worse than arming neither.
//!
//! The panic lands on the worker, not the test thread, so every test below
//! observes it through `JoinHandle::join()` returning `Err` — a bare
//! `assert_eq!` on the signal afterwards would report "0 != 7" and hide the
//! mechanism.
//!
//! Requires the `embed` (or `gpu`) feature:
//!     cargo test -p rinch --features embed --test embed_cross_thread

#![cfg(any(feature = "gpu", feature = "embed"))]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::ThreadId;

use rinch::embed::{RinchContext, RinchContextConfig};
use rinch::platform::{AppAction, PlatformEvent};
use rinch::prelude::*;

// ── harness ──────────────────────────────────────────────────────────────────

type Job = Box<dyn FnOnce() + Send>;

/// Run `f` on the single shared "UI" worker thread and propagate its panic (if
/// any) back to the calling test thread.
///
/// `RinchContext::new` calls `rinch_core::register_main_thread()`, a
/// process-wide `OnceLock`: the first test thread to create a context becomes
/// "the main thread" forever. libtest gives each `#[test]` its own thread, so
/// all bodies are marshalled onto one long-lived worker (which also serializes
/// them). Same reasoning — and same helper — as `tests/multi_context.rs` and
/// `tests/input_routing.rs`.
fn on_ui_thread<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    static SENDER: OnceLock<Mutex<mpsc::Sender<Job>>> = OnceLock::new();
    let sender = SENDER.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("rinch-test-ui".into())
            .spawn(move || {
                for job in rx {
                    job();
                }
            })
            .expect("spawn ui worker");
        Mutex::new(tx)
    });

    let (result_tx, result_rx) = mpsc::channel();
    let job: Job = Box::new(move || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
        let _ = result_tx.send(result);
    });
    sender
        .lock()
        .expect("ui sender lock")
        .send(job)
        .expect("ui worker alive");
    match result_rx.recv().expect("ui worker responded") {
        Ok(v) => v,
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

fn cfg() -> RinchContextConfig {
    RinchContextConfig {
        width: 800,
        height: 600,
        scale_factor: 1.0,
        theme: None,
        fonts: Vec::new(),
    }
}

/// All text content in a context's document, in tree order.
fn doc_text(ctx: &RinchContext) -> String {
    let doc = ctx.app().doc().expect("context has a document").clone();
    let d = doc.borrow();
    rinch_dom::testing::get_text_content(&d.tree, d.tree.root_id)
}

/// A context whose only content is `label: {value}`, kept live by a reactive
/// closure — so "the write arrived" is visible in the rendered document and not
/// only in the signal store.
fn labelled_context(label: &'static str, value: Signal<i32>) -> RinchContext {
    RinchContext::new(cfg(), move |__scope: &mut RenderScope| {
        rsx! {
            div {
                style: "width: 800px; height: 600px;",
                {label} ":" {move || value.get().to_string()}
            }
        }
    })
}

/// Run `f` on a genuinely different thread and fail loudly if it panicked
/// there — which is exactly how the #172 bug presented.
fn on_worker(f: impl FnOnce() + Send + 'static) {
    let handle = std::thread::Builder::new()
        .name("rinch-test-worker".into())
        .spawn(f)
        .expect("spawn worker");
    handle.join().expect(
        "the background thread panicked — a cross-thread write from an embedded \
         context reached `dispatch_to_main_thread` with no dispatcher registered \
         (issue #172)",
    );
}

// ── 1. Signal::send from a worker ────────────────────────────────────────────

/// The headline case. A worker thread's `Signal::send()` must not panic, must
/// not run inline (the signal store is thread-local, so the write has to be
/// marshalled), and must land — in the signal *and* in the document — on the
/// next `update()`.
#[test]
fn background_send_lands_on_the_next_update() {
    on_ui_thread(|| {
        let count = Signal::new(0);
        let mut ctx = labelled_context("n", count);
        assert!(
            doc_text(&ctx).contains("n:0"),
            "initial: {}",
            doc_text(&ctx)
        );

        on_worker(move || count.send(7));

        // Queued, not applied: nothing on the main thread has run yet.
        assert_eq!(
            count.get(),
            0,
            "the write must be queued for the main thread, not applied from the worker"
        );
        assert!(doc_text(&ctx).contains("n:0"));

        let actions = ctx.update(&[]);

        assert_eq!(count.get(), 7, "update() must drain the queued write");
        assert!(
            doc_text(&ctx).contains("n:7"),
            "the reactive text must have been patched: {}",
            doc_text(&ctx)
        );
        assert!(
            actions.contains(&AppAction::RequestRedraw),
            "the host must be told to repaint the frame the write changed: {actions:?}"
        );
    });
}

/// `update_send` is the other half of the documented cross-thread pair and takes
/// a different route into the dispatcher (it marshals a closure, not a value).
#[test]
fn background_update_send_lands_on_the_next_update() {
    on_ui_thread(|| {
        let count = Signal::new(10);
        let mut ctx = labelled_context("n", count);

        on_worker(move || count.update_send(|n| *n += 5));

        assert_eq!(count.get(), 10);
        ctx.update(&[]);
        assert_eq!(count.get(), 15);
        assert!(
            doc_text(&ctx).contains("n:15"),
            "rendered: {}",
            doc_text(&ctx)
        );
    });
}

// ── 2. the generic transport ─────────────────────────────────────────────────

/// `run_on_main_thread` is the transport every off-thread rinch API rides —
/// `set_timeout`'s timer thread, `rinch-http`'s completion hop, `rinch-ws`'s
/// event delivery — so pinning it pins all of them for embed. The closure must
/// run, and it must run on the UI thread rather than the worker.
#[test]
fn run_on_main_thread_from_a_worker_runs_on_the_ui_thread() {
    on_ui_thread(|| {
        let ui_thread = std::thread::current().id();
        let ran_on: Arc<Mutex<Option<ThreadId>>> = Arc::new(Mutex::new(None));
        let mut ctx = labelled_context("n", Signal::new(0));

        let seen = ran_on.clone();
        on_worker(move || {
            rinch_core::run_on_main_thread(move || {
                *seen.lock().unwrap() = Some(std::thread::current().id());
            });
        });

        assert!(
            ran_on.lock().unwrap().is_none(),
            "the closure must not run on the worker that queued it"
        );

        ctx.update(&[]);

        assert_eq!(
            *ran_on.lock().unwrap(),
            Some(ui_thread),
            "update() must have run the queued closure, on the UI thread"
        );
    });
}

// ── 3. multiple contexts share one queue ─────────────────────────────────────

/// The main-thread queue is process-global, so a drain by context A runs work
/// queued against context B. That is correct, not merely tolerable: the payload
/// targets its own signals, signals are thread-local rather than per-document,
/// and B is still repainted by its own `update()` because a signal change
/// notifies every subscriber (#134).
///
/// This pins both halves — A's drain applies the write, and B still reports the
/// redraw — because "shared queue" would otherwise be satisfiable by B silently
/// never repainting.
#[test]
fn a_drain_by_one_context_still_repaints_the_other() {
    on_ui_thread(|| {
        let count = Signal::new(0);
        let mut a = labelled_context("a", Signal::new(0));
        let mut b = labelled_context("b", count);

        // Settle both so the assertions below react to the worker's write only.
        a.update(&[]);
        b.update(&[]);

        on_worker(move || count.send(3));

        // A drains it even though the write targets B's document.
        let a_actions = a.update(&[]);
        assert_eq!(count.get(), 3, "context A's update() must drain the queue");
        assert!(
            doc_text(&b).contains("b:3"),
            "B's reactive text is patched when the write applies, whoever drained it: {}",
            doc_text(&b)
        );
        assert!(
            !doc_text(&a).contains("b:"),
            "A renders only its own document: {}",
            doc_text(&a)
        );
        let _ = a_actions;

        // ...and B is still told to repaint, on its own next frame.
        let b_actions = b.update(&[]);
        assert!(
            b_actions.contains(&AppAction::RequestRedraw),
            "B must still be asked to repaint the write it rendered: {b_actions:?}"
        );
    });
}

// ── 4. the drain is per-frame, not per-event ─────────────────────────────────

/// The drain sits at the top of `update()`, before events, so a write that
/// arrives while the host is idle still lands on the next frame — an embedded
/// host with no input at all must not stall a background thread's updates.
#[test]
fn an_idle_context_still_drains() {
    on_ui_thread(|| {
        let fired = Arc::new(AtomicBool::new(false));
        let flag = fired.clone();
        let mut ctx = labelled_context("n", Signal::new(0));

        on_worker(move || {
            rinch_core::run_on_main_thread(move || flag.store(true, Ordering::Release));
        });

        // No events at all — the empty-slice frame a host pumps while idle.
        let _ = ctx.update(&[] as &[PlatformEvent]);
        assert!(
            fired.load(Ordering::Acquire),
            "an event-free update() must still drain the main-thread queue"
        );
    });
}
