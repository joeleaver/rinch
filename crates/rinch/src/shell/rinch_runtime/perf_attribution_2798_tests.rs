//! #2798: a window's `[PERF]` frame carries the reactive work of its own
//! document, not the thread's. Work under no document is the pass's.

use super::*;
use rinch_core::reactive::{Effect, Signal};
use rinch_dom::perf::Counter;

thread_local! {
    /// The signal each [`counting`] window made, in the order they were
    /// built, for a test to write.
    static SIGNALS: std::cell::RefCell<Vec<Signal<u32>>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// A window whose content owns one signal and one effect that reads it.
fn counting(scope: &mut RenderScope) -> NodeHandle {
    let s = Signal::new(0u32);
    Effect::new(move || {
        let _ = s.get();
    });
    SIGNALS.with(|v| v.borrow_mut().push(s));
    scope.create_element("div")
}

fn signal(i: usize) -> Signal<u32> {
    SIGNALS.with(|v| v.borrow()[i])
}

/// Paint window `id` once: one performance frame, returned.
fn frame(runtime: &mut RinchRuntime, id: u64) -> rinch_dom::perf::FrameStats {
    let w = runtime.w(WindowRole::Other(id));
    w.paint_window().unwrap();
    w.app.last_frame_perf()
}

fn reactive(f: &rinch_dom::perf::FrameStats) -> (u64, u64, u64) {
    (
        f.get(Counter::EffectRuns),
        f.get(Counter::SignalNotifies),
        f.get(Counter::RerenderEventsQueued),
    )
}

/// Two windows, a write that wakes only the first: the first window's frame
/// carries the effect run and the write, the second's cached frame carries
/// nothing, and a write under no document is the pass's alone.
#[test]
fn a_windows_frame_carries_only_its_own_reactive_work_2798() {
    // The runtime holds `RERENDER_EVENTS_TEST_LOCK` while it lives, so the
    // `ReRender` count is this test's alone.
    SIGNALS.with(|v| v.borrow_mut().clear());
    let mut runtime = runtime(crate::ExitWhen::MainWindowCloses);
    let (_, a) = open(&mut runtime, counting);
    let (_, b) = open(&mut runtime, counting);
    // Settle: the mounts' work is in these frames, and the pass.
    frame(&mut runtime, a);
    frame(&mut runtime, b);
    runtime.main.app.end_perf_frame();
    crate::window_perf::end_pass();
    let _ = NATIVE_EVENT_QUEUE.lock().unwrap().drain();

    // A write in window a's dispatch wakes a's effect three times over; it
    // also queues a's re-render.
    {
        let _d = rinch_core::push_dispatching_doc(doc_of(&runtime, a));
        for n in 1..=3 {
            signal(0).set(n);
        }
        send_native_event(RinchNativeEvent::ReRender);
    }
    let _ = NATIVE_EVENT_QUEUE.lock().unwrap().drain();
    // A write from outside any document, two of them: nobody's.
    let unread = Signal::new(0u32);
    unread.set(1);
    unread.set(2);

    assert_eq!(reactive(&frame(&mut runtime, a)), (3, 3, 1), "window a");
    assert_eq!(reactive(&frame(&mut runtime, b)), (0, 0, 0), "window b");
    let main = runtime.main.app.end_perf_frame().unwrap();
    assert_eq!(reactive(&main), (0, 0, 0), "the main window");

    let pass = crate::window_perf::end_pass().expect("the pass had work");
    assert_eq!(
        reactive(&pass),
        (0, 2, 0),
        "the pass: the writes under no document"
    );
    assert_eq!(crate::perf::pass().last_pass, pass);
    // Nothing since: no pass line.
    assert_eq!(crate::window_perf::end_pass(), None);
}

/// Window b's effect, woken by a write in window a's dispatch, is b's run:
/// the write is a's, the run is b's.
#[test]
fn an_effect_run_is_its_own_documents_whoever_wrote_2798() {
    SIGNALS.with(|v| v.borrow_mut().clear());
    let mut runtime = runtime(crate::ExitWhen::MainWindowCloses);
    let (_, a) = open(&mut runtime, counting);
    let (_, b) = open(&mut runtime, counting);
    frame(&mut runtime, a);
    frame(&mut runtime, b);
    crate::window_perf::end_pass();
    {
        let _d = rinch_core::push_dispatching_doc(doc_of(&runtime, a));
        signal(1).set(7);
    }
    let fa = frame(&mut runtime, a);
    let fb = frame(&mut runtime, b);
    assert_eq!(
        (fa.get(Counter::EffectRuns), fa.get(Counter::SignalNotifies)),
        (0, 1),
        "window a wrote"
    );
    assert_eq!(
        (fb.get(Counter::EffectRuns), fb.get(Counter::SignalNotifies)),
        (1, 0),
        "window b's effect ran"
    );
}

/// One window on the thread reads as before: work under no document is on
/// its line, since it and the thread are the same.
#[test]
fn a_single_window_still_carries_the_threads_work_2798() {
    SIGNALS.with(|v| v.borrow_mut().clear());
    let mut runtime = runtime(crate::ExitWhen::MainWindowCloses);
    runtime.main.app.end_perf_frame();
    let unread = Signal::new(0u32);
    unread.set(1);
    let f = runtime.main.app.end_perf_frame().unwrap();
    assert_eq!(f.get(Counter::SignalNotifies), 1);
    assert_eq!(crate::window_perf::end_pass(), None, "no pass line");
}

/// A windowless app's root ends no frames, so its document is not counted
/// apart: an app with one window reads as one, the window's frames carrying
/// the work of nobody.
#[test]
fn a_windowless_apps_only_window_carries_the_threads_work_2798() {
    let mut runtime = windowless(root_opening_a_window);
    let opened = crate::windows();
    let id = make(&mut runtime, opened[0]);
    frame(&mut runtime, id);
    let unread = Signal::new(0u32);
    unread.set(1);
    assert_eq!(frame(&mut runtime, id).get(Counter::SignalNotifies), 1);
    assert_eq!(crate::window_perf::end_pass(), None, "no pass line");
}
