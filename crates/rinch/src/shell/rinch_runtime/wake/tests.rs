//! The native event queue's and the wake's unit tests (moved out of `wake.rs`).

use super::*;

fn rerenders(events: &[(EventFor, RinchNativeEvent)]) -> usize {
    events
        .iter()
        .filter(|(_, e)| matches!(e, RinchNativeEvent::ReRender))
        .count()
}

/// Any number of `ReRender`s between two drains queue one event, and only
/// the first asks for a wake.
#[test]
fn rerenders_coalesce_until_the_queue_is_drained() {
    let mut q = NativeEventQueue::new();
    let accepted: Vec<bool> = (0..7)
        .map(|_| q.push(EventFor::Sender(None), RinchNativeEvent::ReRender))
        .collect();
    assert_eq!(accepted, [true, false, false, false, false, false, false]);
    assert_eq!(rerenders(&q.drain()), 1);

    // Drained: the next one queues again, and again coalesces its followers.
    assert!(q.push(EventFor::Sender(None), RinchNativeEvent::ReRender));
    assert!(!q.push(EventFor::Sender(None), RinchNativeEvent::ReRender));
    assert_eq!(rerenders(&q.drain()), 1);
    assert!(q.is_empty());
}

/// Through the real entry point: `rerender_events_queued` (what
/// `perf_stats` reports) counts the events actually queued, so a burst of
/// signal flushes between two drains counts one. The only test in the
/// process that sends a `ReRender` through the static queue.
#[test]
fn send_native_event_counts_one_rerender_per_drain() {
    use std::sync::atomic::Ordering;
    let _lock = crate::app::RERENDER_EVENTS_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _ = NATIVE_EVENT_QUEUE.lock().unwrap().drain();
    let before = crate::app::RERENDER_EVENTS_QUEUED.load(Ordering::Relaxed);
    for _ in 0..6 {
        send_native_event(RinchNativeEvent::ReRender);
    }
    assert_eq!(
        crate::app::RERENDER_EVENTS_QUEUED.load(Ordering::Relaxed) - before,
        1
    );
    assert_eq!(rerenders(&NATIVE_EVENT_QUEUE.lock().unwrap().drain()), 1);
    send_native_event(RinchNativeEvent::ReRender);
    assert_eq!(
        crate::app::RERENDER_EVENTS_QUEUED.load(Ordering::Relaxed) - before,
        2,
        "a drain re-arms it"
    );
    let _ = NATIVE_EVENT_QUEUE.lock().unwrap().drain();
}

/// Issue #988. A second thread's `run_on_main_thread` lands between a
/// wake's two halves: after the main-thread queue was emptied (so
/// `queue_main_callback` answers "was empty" and it sends a `ReRender`)
/// and before the native drain (so that `ReRender` coalesces into the one
/// that caused this wake, and asks for no wake of its own). The drain then
/// consumes the old `ReRender`, and nothing is left to wake the loop for
/// the new callback.
///
/// Two injection points, both inside that window: the end of the queued
/// half, and the start of the native half (before its drain takes the
/// lock). The second is what a fix that samples "is anything queued?"
/// *before* the native drain gets wrong.
///
/// The invariant: once the wake returns, the injected callback has either
/// run or has a wake owed (a `ReRender` in the queue — which is what
/// `send_native_event` wakes the loop for). "Or has run" keeps the fixture
/// honest if another test in this process drains the shared main queue in
/// the meantime; that drain runs it, which is not a lost wake.
fn callback_queued_mid_wake_keeps_its_wake(inject_in_native_half: bool) {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    let _lock = crate::app::RERENDER_EVENTS_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _ = NATIVE_EVENT_QUEUE.lock().unwrap().drain();
    // The wake being served: one queued callback and its ReRender.
    run_on_main_thread(|| {});
    NATIVE_EVENT_QUEUE
        .lock()
        .unwrap()
        .push(EventFor::Sender(None), RinchNativeEvent::ReRender);

    let injected_ran = Arc::new(AtomicBool::new(false));
    let flag = injected_ran.clone();
    // Another thread, inside the window.
    let inject = move || {
        std::thread::spawn(move || run_on_main_thread(move || flag.store(true, Ordering::SeqCst)))
            .join()
            .unwrap();
    };
    let mut inject = Some(inject);
    drain_wake_queues(
        &mut inject,
        |inject| {
            rinch_core::drain_main_callbacks();
            if !inject_in_native_half {
                (inject.take().unwrap())();
            }
        },
        |inject| {
            if inject_in_native_half {
                (inject.take().unwrap())();
            }
            let _ = NATIVE_EVENT_QUEUE.lock().unwrap().drain();
        },
    );
    assert!(inject.is_none(), "the injection ran");

    let wake_owed = rerenders(&NATIVE_EVENT_QUEUE.lock().unwrap().drain()) > 0;
    if !wake_owed {
        // Only another test's drain may have taken it; give that drain
        // time to finish running it.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
        while !injected_ran.load(Ordering::SeqCst) && std::time::Instant::now() < deadline {
            std::thread::yield_now();
        }
    }
    let ran = injected_ran.load(Ordering::SeqCst);
    // Leave the shared queue as we found it.
    rinch_core::drain_main_callbacks();
    assert!(
        wake_owed || ran,
        "the callback queued mid-wake is stranded: still queued, no wake owed"
    );
}

#[test]
fn a_callback_queued_after_the_callback_drain_keeps_its_wake() {
    callback_queued_mid_wake_keeps_its_wake(false);
}

#[test]
fn a_callback_queued_as_the_native_drain_starts_keeps_its_wake() {
    callback_queued_mid_wake_keeps_its_wake(true);
}

/// Issue #1733: what made the two tests above flaky. Dropping a
/// [`RinchRuntime`] runs its shutdown, which **drops** every queued
/// main-thread callback without running it (`clear_main_callbacks`) — and
/// the runtime-building tests in this binary did that on other threads
/// while a wake test's injected callback sat in the shared queue: neither
/// run nor owed a wake. A real process has one runtime, dropped at exit,
/// so only the test binary had the race. Every test runtime now holds
/// `RERENDER_EVENTS_TEST_LOCK` for its lifetime (`RinchRuntime::_queue_lock`).
///
/// The runtime here is built and dropped on another thread, inside the
/// wake test's window: before the fix its drop landed there every time.
#[test]
fn a_runtime_dropped_on_another_thread_mid_wake_strands_nothing() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    let _lock = crate::app::RERENDER_EVENTS_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _ = NATIVE_EVENT_QUEUE.lock().unwrap().drain();
    rinch_core::drain_main_callbacks();
    run_on_main_thread(|| {});
    let injected_ran = Arc::new(AtomicBool::new(false));
    let flag = injected_ran.clone();
    let dropped = Arc::new(AtomicBool::new(false));
    let dropped_flag = dropped.clone();
    let mut dropper = None;
    drain_wake_queues(
        &mut (),
        |_| {
            rinch_core::drain_main_callbacks();
            run_on_main_thread(move || flag.store(true, Ordering::SeqCst));
            dropper = Some(std::thread::spawn(move || {
                drop(RinchRuntime::new(
                    "t",
                    800,
                    600,
                    |s: &mut RenderScope| s.create_element("div"),
                    &[],
                ));
                dropped_flag.store(true, Ordering::SeqCst);
            }));
            // Long enough for an unserialised runtime to be built and
            // dropped; a serialised one waits for this test's lock.
            let deadline = std::time::Instant::now() + std::time::Duration::from_millis(500);
            while !dropped.load(Ordering::SeqCst) && std::time::Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        },
        |_| {
            let _ = NATIVE_EVENT_QUEUE.lock().unwrap().drain();
        },
    );
    let wake_owed = rerenders(&NATIVE_EVENT_QUEUE.lock().unwrap().drain()) > 0;
    let ran = injected_ran.load(Ordering::SeqCst);
    rinch_core::drain_main_callbacks();
    drop(_lock);
    dropper.unwrap().join().unwrap();
    assert!(
        wake_owed || ran,
        "a runtime dropped mid-wake threw away the queued callback"
    );
}

/// Why every dispatch asks for a `ReRender` (#1540) rather than only a push
/// onto an empty main queue: the queue can be non-empty with no wake owed
/// for it — a closure pushed past the dispatcher (#1035's shape), or one
/// queued plainly before the dispatcher existed (#1541) — and under the
/// empty-queue rule every later sender, from any thread, then found it
/// non-empty and asked for nothing. Run through both senders the shell
/// installs: the dispatcher behind `Signal::send`, and the shell's own
/// `run_on_main_thread`.
fn a_dispatch_behind_an_unwoken_callback_still_wakes(send: fn()) {
    let _lock = crate::app::RERENDER_EVENTS_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _ = NATIVE_EVENT_QUEUE.lock().unwrap().drain();
    rinch_core::drain_main_callbacks();
    // A callback with no wake of its own.
    rinch_core::queue_main_callback(Box::new(|| {}));
    // The next write from a worker.
    std::thread::spawn(send).join().unwrap();
    let wake_owed = rerenders(&NATIVE_EVENT_QUEUE.lock().unwrap().drain()) > 0;
    let still_queued = rinch_core::main_callbacks_pending();
    rinch_core::drain_main_callbacks();
    assert!(
        wake_owed || !still_queued,
        "cross-thread writes stranded: queued, and no wake owed"
    );
}

#[test]
fn a_signal_send_behind_an_unwoken_callback_still_wakes() {
    a_dispatch_behind_an_unwoken_callback_still_wakes(|| dispatch_to_main_thread(Box::new(|| {})));
}

#[test]
fn a_run_on_main_thread_behind_an_unwoken_callback_still_wakes() {
    a_dispatch_behind_an_unwoken_callback_still_wakes(|| run_on_main_thread(|| {}));
}

/// Issue #1540's fix asks for a `ReRender` on every dispatch; the
/// coalescing #988 / #1035 rely on still holds: a burst of dispatches
/// between two native drains leaves exactly one pending `ReRender`, and
/// counts one in `rerender_events_queued`.
#[test]
fn a_burst_of_dispatches_leaves_one_rerender_per_drain() {
    use std::sync::atomic::Ordering;
    let _lock = crate::app::RERENDER_EVENTS_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _ = NATIVE_EVENT_QUEUE.lock().unwrap().drain();
    for round in 0..2 {
        let before = crate::app::RERENDER_EVENTS_QUEUED.load(Ordering::Relaxed);
        std::thread::spawn(|| {
            for _ in 0..25 {
                dispatch_to_main_thread(Box::new(|| {}));
                run_on_main_thread(|| {});
            }
        })
        .join()
        .unwrap();
        assert_eq!(
            crate::app::RERENDER_EVENTS_QUEUED.load(Ordering::Relaxed) - before,
            1,
            "round {round}: one ReRender queued for 50 dispatches"
        );
        assert_eq!(rerenders(&NATIVE_EVENT_QUEUE.lock().unwrap().drain()), 1);
    }
    rinch_core::drain_main_callbacks();
}

/// Issue #1541: a callback queued plainly before the dispatcher existed
/// is given a `ReRender`, and the wake is paid, once the proxy is set.
#[test]
fn work_queued_before_the_proxy_is_owed_a_wake() {
    let _lock = crate::app::RERENDER_EVENTS_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _ = NATIVE_EVENT_QUEUE.lock().unwrap().drain();
    // Queued plainly, as `dispatch_main_callback` does with no dispatcher.
    rinch_core::queue_main_callback(Box::new(|| {}));
    let mut woke = 0;
    owe_startup_wake(|| woke += 1);
    let owed = rerenders(&NATIVE_EVENT_QUEUE.lock().unwrap().drain());
    let still_queued = rinch_core::main_callbacks_pending();
    rinch_core::drain_main_callbacks();
    assert!(
        (owed == 1 && woke == 1) || !still_queued,
        "queued before the proxy, and no wake owed or paid (owed {owed}, woke {woke})"
    );
}

/// Issue #1540: a `ReRender` sent before the proxy existed (the mount's
/// signal-change callback, an early `Signal::send`) carried no wake and
/// holds the coalescing flag up, so every later `ReRender` coalesces into
/// it and wakes nothing. A window that paints drains it; a windowless or
/// hidden app never does. The wake owed once the proxy is set is what
/// clears it — and it is the `wake` call, not a further `ReRender`, that
/// does: one more `ReRender` coalesces into the stale one.
#[test]
fn a_rerender_queued_before_the_proxy_is_woken_for() {
    let _lock = crate::app::RERENDER_EVENTS_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _ = NATIVE_EVENT_QUEUE.lock().unwrap().drain();
    rinch_core::drain_main_callbacks();
    // The stale `ReRender`, flag up, nothing woken.
    send_native_event(RinchNativeEvent::ReRender);
    // A worker's write behind it: coalesced, no wake of its own.
    std::thread::spawn(|| dispatch_to_main_thread(Box::new(|| {})))
        .join()
        .unwrap();
    let mut woke = 0;
    let owed = owe_startup_wake(|| woke += 1);
    let _ = NATIVE_EVENT_QUEUE.lock().unwrap().drain();
    rinch_core::drain_main_callbacks();
    assert!(owed, "a ReRender is waiting");
    assert_eq!(woke, 1, "and the loop is woken for it");
}

/// Only `ReRender` coalesces: every other event is a distinct request, and
/// the `ReRender` keeps the position it was first queued at.
#[test]
fn other_events_are_never_coalesced() {
    let mut q = NativeEventQueue::new();
    assert!(q.push(EventFor::Sender(None), RinchNativeEvent::MinimizeWindow));
    assert!(q.push(EventFor::Sender(None), RinchNativeEvent::ReRender));
    assert!(q.push(EventFor::Sender(None), RinchNativeEvent::MinimizeWindow));
    assert!(!q.push(EventFor::Sender(None), RinchNativeEvent::ReRender));
    assert!(q.push(EventFor::Sender(None), RinchNativeEvent::ShowWindow));
    let drained = q.drain();
    assert_eq!(drained.len(), 4, "{drained:?}");
    assert!(matches!(
        drained[1],
        (EventFor::Sender(None), RinchNativeEvent::ReRender)
    ));
    assert_eq!(rerenders(&drained), 1);
}

/// An event carries the document whose code sent it (#2416), so the
/// runtime can give it to that document's window; from outside any
/// document it carries none.
#[test]
fn an_event_carries_the_document_that_sent_it() {
    let _lock = crate::app::RERENDER_EVENTS_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _ = NATIVE_EVENT_QUEUE.lock().unwrap().drain();
    send_native_event(RinchNativeEvent::MinimizeWindow);
    {
        let _doc = rinch_core::push_dispatching_doc(42);
        send_native_event(RinchNativeEvent::HideWindow);
    }
    send_native_event_for(EventFor::Window(7), RinchNativeEvent::CloseWindow);
    let drained = NATIVE_EVENT_QUEUE.lock().unwrap().drain();
    assert!(matches!(
        drained[0],
        (EventFor::Sender(None), RinchNativeEvent::MinimizeWindow)
    ));
    assert!(matches!(
        drained[1],
        (EventFor::Sender(Some(42)), RinchNativeEvent::HideWindow)
    ));
    assert!(matches!(
        drained[2],
        (EventFor::Window(7), RinchNativeEvent::CloseWindow)
    ));
}

/// The second look for an exit (#3015), as `try_run_rinch_with_window_props_and_menu`
/// makes it: an exit sent from another thread after `App::try_run` looked
/// and while the runtime was made (the display connected to, the app
/// mounted) ends the run before the event loop runs, with `Ok(())`, the
/// runtime dropped; and the run's end leaves the queue as before any run.
#[test]
fn an_exit_while_the_display_connects_ends_the_run_before_the_loop() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    struct Runtime(Arc<AtomicBool>);
    impl Drop for Runtime {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    let queue = Arc::new(std::sync::Mutex::new(NativeEventQueue::with_drainer(
        Drainer::Coming,
    )));
    // `App::try_run`'s first look: nothing asked yet.
    assert!(!queue.lock().unwrap().take_exit());
    // `RinchRuntime::with_app`, while the display is connected to.
    queue.lock().unwrap().note_runtime();
    // The signal-watching thread's exit, in the window between the looks.
    let sender = Arc::clone(&queue);
    std::thread::spawn(move || {
        sender
            .lock()
            .unwrap()
            .push(EventFor::Sender(None), RinchNativeEvent::Exit);
    })
    .join()
    .unwrap();

    let dropped = Arc::new(AtomicBool::new(false));
    let mut looped = false;
    let result = run_unless_exit_asked_in(&queue, Runtime(Arc::clone(&dropped)), |_| {
        looped = true;
        Ok(())
    });
    assert!(result.is_ok(), "{result:?}");
    assert!(!looped, "the event loop ran for a run asked to exit");
    assert!(
        dropped.load(Ordering::SeqCst),
        "the runtime was not dropped"
    );
    let q = queue.lock().unwrap();
    assert!(q.is_empty());
    assert_eq!(q.drainer(), Drainer::Coming, "the run's end was not noted");
}

/// Without an exit the loop runs, its result is the run's, and what the
/// runtime left queued when its loop ended (a second exit racing the first)
/// ends with it rather than ending the next run (#3015).
#[test]
fn without_an_exit_the_loop_runs_and_its_leftovers_end_with_it() {
    let queue = std::sync::Mutex::new(NativeEventQueue::with_drainer(Drainer::Coming));
    queue.lock().unwrap().note_runtime();
    let result = run_unless_exit_asked_in(&queue, (), |()| {
        let mut q = queue.lock().unwrap();
        q.push(EventFor::Sender(None), RinchNativeEvent::Exit);
        q.push(EventFor::Sender(None), RinchNativeEvent::Exit);
        let _ = q.drain().len();
        // The second exit, sent as the loop stopped.
        q.push(EventFor::Sender(None), RinchNativeEvent::Exit);
        Err(crate::RunError::DisplayLost("gone".into()))
    });
    assert!(
        matches!(result, Err(crate::RunError::DisplayLost(_))),
        "{result:?}"
    );
    let mut q = queue.lock().unwrap();
    assert_eq!(q.drainer(), Drainer::Coming);
    assert!(
        !q.take_exit(),
        "the ended run's exit would end the next run"
    );
}
