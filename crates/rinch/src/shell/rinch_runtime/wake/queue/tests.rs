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

/// A `RinchContext` made with no runtime: what was kept is dropped but the
/// exit, and every later request too but one exit (#2801, #3015): the exit
/// is for a desktop runtime the process may still run.
#[test]
#[cfg(any(feature = "gpu-renderer", feature = "embed"))]
fn an_embed_host_keeps_an_exit_and_nothing_else() {
    let mut q = before_run();
    q.push(MAIN, RinchNativeEvent::ShowWindow);
    q.push(MAIN, RinchNativeEvent::Exit);
    q.push(MAIN, RinchNativeEvent::HideWindow);
    q.note_embed_host();
    assert_eq!(q.drainer(), Drainer::EmbedHost);
    assert_eq!(
        (q.events.len(), exits(&q)),
        (1, 1),
        "the exit alone is kept"
    );
    assert!(!q.push(MAIN, RinchNativeEvent::ReRender));
    assert!(!q.push(MAIN, RinchNativeEvent::Exit), "a second exit");
    assert_eq!(q.events.len(), 1);
    // The run that comes takes it.
    assert!(q.take_exit());
    assert!(q.is_empty());
}

/// An exit sent after a context was made and before the run (an app that
/// renders offscreen, then runs) ends that run (#3015).
#[test]
#[cfg(any(feature = "gpu-renderer", feature = "embed"))]
fn an_exit_between_a_context_and_the_run_is_taken() {
    let mut q = before_run();
    q.note_embed_host();
    assert!(!q.take_exit(), "nothing asked yet");
    assert!(q.push(MAIN, RinchNativeEvent::Exit));
    assert!(q.take_exit());
    assert!(!q.take_exit(), "taken once");
}

/// An embed-only process holds at most the one exit, however much it sends
/// from outside its contexts: nothing grows, and nothing but a run's start
/// reads it.
#[test]
#[cfg(any(feature = "gpu-renderer", feature = "embed"))]
fn an_embed_only_process_holds_at_most_one_exit() {
    let mut q = before_run();
    q.note_embed_host();
    for _ in 0..3 * KEPT_BEFORE_RUN {
        q.push(MAIN, RinchNativeEvent::Exit);
        q.push(MAIN, RinchNativeEvent::ShowWindow);
        q.push(MAIN, RinchNativeEvent::ReRender);
    }
    assert_eq!((q.events.len(), exits(&q)), (1, 1));
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

/// When a run ends, what its runtime left queued ends with it: a second exit
/// sent as the loop stopped does not end the next run. The queue is as
/// before the first run again: an exit sent between two runs ends the
/// second, and what is kept is bounded again.
#[test]
fn a_runs_leftovers_end_with_it() {
    let mut q = before_run();
    q.note_runtime();
    q.push(MAIN, RinchNativeEvent::Exit);
    q.push(MAIN, RinchNativeEvent::ShowWindow);
    q.push(MAIN, RinchNativeEvent::ReRender);
    q.note_run_ended();
    assert_eq!(q.drainer(), Drainer::Coming);
    assert!(q.is_empty(), "the ended run's requests are still queued");
    assert!(
        !q.take_exit(),
        "the ended run's exit would end the next one"
    );
    // A ReRender queues again: the ended run's pending one is gone.
    assert!(q.push(MAIN, RinchNativeEvent::ReRender));

    for _ in 0..KEPT_BEFORE_RUN + 10 {
        q.push(MAIN, RinchNativeEvent::ShowWindow);
    }
    assert_eq!(
        q.events.len(),
        KEPT_BEFORE_RUN,
        "bounded again between runs"
    );
    assert!(q.push(MAIN, RinchNativeEvent::Exit));
    assert!(q.take_exit(), "an exit between two runs ends the second");
}

/// A run ended in a process that made a context is an embed host again.
#[test]
#[cfg(any(feature = "gpu-renderer", feature = "embed"))]
fn a_run_ended_after_a_context_returns_to_the_embed_host() {
    let mut q = before_run();
    q.note_embed_host();
    q.note_runtime();
    q.note_run_ended();
    assert_eq!(q.drainer(), Drainer::EmbedHost);
    assert!(!q.push(MAIN, RinchNativeEvent::ShowWindow));
    assert!(q.push(MAIN, RinchNativeEvent::Exit));
}

/// Emptying the queue returns its memory: a process that kept
/// [`KEPT_BEFORE_RUN`] requests does not hold their buffer for its life.
#[test]
fn clearing_returns_the_queues_memory() {
    let mut q = before_run();
    for _ in 0..KEPT_BEFORE_RUN {
        q.push(MAIN, RinchNativeEvent::ShowWindow);
    }
    q.push(MAIN, RinchNativeEvent::Exit);
    assert!(q.take_exit());
    assert_eq!(q.events.capacity(), 0, "an exit's clear kept the buffer");

    q.note_runtime();
    for _ in 0..100 {
        q.push(MAIN, RinchNativeEvent::ShowWindow);
    }
    q.note_run_ended();
    assert_eq!(q.events.capacity(), 0, "a run's end kept the buffer");
}
