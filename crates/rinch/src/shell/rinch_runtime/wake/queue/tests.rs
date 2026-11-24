//! What the queue does with an event before a desktop runtime exists, once
//! one does, and in an embed host (#2801, #3015).

use super::*;

const MAIN: EventFor = EventFor::Sender(None);

fn before_run() -> NativeEventQueue {
    NativeEventQueue::with_drainer(Drainer::Coming)
}

fn exits(q: &NativeEventQueue) -> usize {
    q.events
        .iter()
        .filter(|(_, e)| matches!(e, RinchNativeEvent::Exit))
        .count()
}

/// Before the run, a request is kept for the runtime to come, and that
/// runtime's first drain delivers it, in order.
#[test]
fn a_request_before_the_run_reaches_the_runtime_that_comes() {
    let mut q = before_run();
    assert!(q.push(MAIN, RinchNativeEvent::SetWindowTitle("early".into())));
    assert!(q.push(MAIN, RinchNativeEvent::HideWindow));
    q.note_runtime();
    assert_eq!(q.drainer(), Drainer::Runtime);
    let drained = q.drain();
    assert!(
        matches!(
            drained.as_slice(),
            [
                (_, RinchNativeEvent::SetWindowTitle(t)),
                (_, RinchNativeEvent::HideWindow)
            ] if t == "early"
        ),
        "{drained:?}"
    );
}

/// An exit before the run is kept, and taking it empties the queue: the run
/// returns, and nothing else kept for it has a window to reach.
#[test]
fn an_exit_before_the_run_is_taken_once() {
    let mut q = before_run();
    assert!(!q.take_exit(), "an exit nobody asked for");
    q.push(MAIN, RinchNativeEvent::ShowWindow);
    assert!(q.push(MAIN, RinchNativeEvent::Exit));
    q.push(MAIN, RinchNativeEvent::HideWindow);
    assert!(q.take_exit());
    assert!(q.is_empty(), "the exited run's requests are still queued");
    assert!(!q.take_exit(), "the exit was answered once already");
}

/// An exit that came in after `App::try_run` looked, while the runtime was
/// being made, is still there for the runtime's own look.
#[test]
fn an_exit_racing_the_runtime_is_still_taken() {
    let mut q = before_run();
    assert!(!q.take_exit());
    q.push(MAIN, RinchNativeEvent::Exit);
    q.note_runtime();
    assert!(q.take_exit());
}

/// A process that never runs keeps at most [`KEPT_BEFORE_RUN`] requests and
/// one exit, however many it sends; the exit is kept past the bound.
#[test]
fn what_is_kept_before_the_run_is_bounded() {
    let mut q = before_run();
    for i in 0..3 * KEPT_BEFORE_RUN {
        q.push(MAIN, RinchNativeEvent::SetWindowTitle(format!("{i}")));
        q.push(MAIN, RinchNativeEvent::ReRender);
    }
    assert_eq!(q.events.len(), KEPT_BEFORE_RUN);
    for _ in 0..10 {
        q.push(MAIN, RinchNativeEvent::Exit);
    }
    assert_eq!(exits(&q), 1, "a repeated exit is kept once");
    assert_eq!(q.events.len(), KEPT_BEFORE_RUN + 1);
    assert!(
        !q.push(MAIN, RinchNativeEvent::HideWindow),
        "a request past the bound asked for a wake"
    );
    // Once a runtime drains it, the bound is no longer the queue's.
    q.note_runtime();
    let _ = q.drain();
    for _ in 0..2 * KEPT_BEFORE_RUN {
        assert!(q.push(MAIN, RinchNativeEvent::HideWindow));
    }
    assert_eq!(q.events.len(), 2 * KEPT_BEFORE_RUN);
}

/// A `RinchContext` made with no runtime: what was kept is dropped, and
/// every later request too, the exit included (#2801).
#[test]
#[cfg(any(feature = "gpu-renderer", feature = "embed"))]
fn an_embed_host_keeps_nothing() {
    let mut q = before_run();
    q.push(MAIN, RinchNativeEvent::ShowWindow);
    q.note_embed_host();
    assert_eq!(q.drainer(), Drainer::EmbedHost);
    assert!(q.is_empty());
    assert!(!q.push(MAIN, RinchNativeEvent::Exit));
    assert!(!q.push(MAIN, RinchNativeEvent::ReRender));
    assert!(q.is_empty());
    assert!(!q.take_exit());
}

/// A context made once a runtime exists changes nothing: the runtime still
/// takes what is sent from outside the context.
#[test]
#[cfg(any(feature = "gpu-renderer", feature = "embed"))]
fn a_context_beside_a_runtime_leaves_its_queue_alone() {
    let mut q = before_run();
    q.note_runtime();
    q.push(MAIN, RinchNativeEvent::ShowWindow);
    q.note_embed_host();
    assert_eq!(q.drainer(), Drainer::Runtime);
    assert!(q.push(MAIN, RinchNativeEvent::HideWindow));
    assert_eq!(q.events.len(), 2);
}

/// A runtime made after a context was still takes over.
#[test]
#[cfg(any(feature = "gpu-renderer", feature = "embed"))]
fn a_runtime_after_a_context_queues_again() {
    let mut q = before_run();
    q.note_embed_host();
    q.note_runtime();
    assert!(q.push(MAIN, RinchNativeEvent::Exit));
    assert!(q.take_exit());
}
