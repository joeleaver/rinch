//! A `Signal::send` to a signal made on a thread that hosts no app is dropped
//! where a host thread drains it, and the warning names that cause, not the
//! freed-signal one (issue #2889).
//!
//! `route_write` queues such a write unrouted (the signal's thread is no host,
//! so nothing would drain a write queued for it). A host thread's drain takes
//! it, finds no signal of that id in its own store (the id carries the other
//! thread's tag) and drops it. Before #2889 that drop said "the scope that
//! owned this signal was disposed", which had not happened, and advised
//! `is_alive()`, which a worker cannot use.
//!
//! Own binary: the queue is one per process, so a host thread of another test
//! here could drain the write first and log it outside this test's capture.

use rinch_core::{Scope, Signal, drain_main_callbacks, mark_host_thread};
use std::sync::{Arc, Mutex};
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::{Event, Metadata, Subscriber};

/// Records the message of every event, for one thread (`set_default`).
#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<String>>>);

struct Message<'a>(&'a mut String);

impl Visit for Message<'_> {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            *self.0 = format!("{value:?}");
        }
    }
}

impl Subscriber for Capture {
    fn enabled(&self, _: &Metadata<'_>) -> bool {
        true
    }
    fn new_span(&self, _: &Attributes<'_>) -> Id {
        Id::from_u64(1)
    }
    fn record(&self, _: &Id, _: &Record<'_>) {}
    fn record_follows_from(&self, _: &Id, _: &Id) {}
    fn event(&self, event: &Event<'_>) {
        let mut message = String::new();
        event.record(&mut Message(&mut message));
        self.0.lock().unwrap().push(message);
    }
    fn enter(&self, _: &Id) {}
    fn exit(&self, _: &Id) {}
}

impl Capture {
    fn messages(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }
}

/// The issue's shape: a bare thread (no app) makes the signal, a worker sends
/// to it, a host thread drains the write.
#[test]
fn a_send_to_a_signal_of_a_thread_that_hosts_no_app_says_so() {
    let capture = Capture::default();
    let seen = capture.clone();
    let messages = std::thread::spawn(move || {
        let _log = tracing::subscriber::set_default(seen.clone());
        mark_host_thread();
        // Kept alive until the end: the signal is never freed.
        let (signal_tx, signal_rx) = std::sync::mpsc::channel();
        let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
        let bare = std::thread::spawn(move || {
            let signal: Signal<u32> = Signal::new(0);
            signal_tx.send(signal).unwrap();
            done_rx.recv().unwrap();
            signal.get()
        });
        let signal = signal_rx.recv().unwrap();
        std::thread::spawn(move || signal.send(7)).join().unwrap();
        drain_main_callbacks();
        done_tx.send(()).unwrap();
        assert_eq!(bare.join().unwrap(), 0, "the write is dropped, as before");
        seen.messages()
    })
    .join()
    .unwrap();

    let dropped: Vec<_> = messages
        .iter()
        .filter(|m| m.contains("the write was dropped"))
        .collect();
    assert_eq!(dropped.len(), 1, "one warning for the drop: {messages:?}");
    let m = dropped[0];
    assert!(
        m.contains("made on another thread") && m.contains("hosts no app"),
        "names the real cause: {m}"
    );
    assert!(m.contains("TestApp"), "says where to make the signal: {m}");
    assert!(
        !m.contains("freed signal") && !m.contains("disposed") && !m.contains("is_alive"),
        "does not name the freed-signal cause or its advice: {m}"
    );
    assert!(
        m.contains("send_to_non_host_signal_2889.rs"),
        "names the send's call site: {m}"
    );
}

/// Control: a write to a signal whose scope was disposed keeps the freed-signal
/// warning.
#[test]
fn a_write_to_a_freed_signal_keeps_the_freed_signal_warning() {
    let capture = Capture::default();
    let messages = {
        let _log = tracing::subscriber::set_default(capture.clone());
        let scope = Scope::new();
        let signal: Signal<u32> = scope.run(|| Signal::new(0));
        scope.dispose();
        signal.set(1);
        capture.messages()
    };
    assert_eq!(messages.len(), 1, "{messages:?}");
    let m = &messages[0];
    assert!(
        m.contains("on a freed signal") && m.contains("was disposed"),
        "the freed-signal warning: {m}"
    );
    assert!(!m.contains("another thread"), "{m}");
}

/// A direct `set` on one host thread to another host thread's signal: that
/// thread does host an app, so the advice is `send`, not where to make it.
#[test]
fn a_direct_write_to_another_host_threads_signal_advises_send() {
    let (signal_tx, signal_rx) = std::sync::mpsc::channel();
    let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
    let owner = std::thread::spawn(move || {
        mark_host_thread();
        let signal: Signal<u32> = Signal::new(0);
        signal_tx.send(signal).unwrap();
        done_rx.recv().unwrap();
        signal.get()
    });
    let signal = signal_rx.recv().unwrap();
    let capture = Capture::default();
    let seen = capture.clone();
    let messages = std::thread::spawn(move || {
        let _log = tracing::subscriber::set_default(seen.clone());
        mark_host_thread();
        signal.set(5);
        seen.messages()
    })
    .join()
    .unwrap();
    done_tx.send(()).unwrap();
    assert_eq!(owner.join().unwrap(), 0, "the write is dropped");

    assert_eq!(messages.len(), 1, "{messages:?}");
    let m = &messages[0];
    assert!(
        m.contains("Signal::set() to a signal made on another thread") && m.contains("send()"),
        "{m}"
    );
    assert!(
        !m.contains("hosts no app") && !m.contains("freed signal"),
        "{m}"
    );
}
