//! The desktop worker: a thread per worker, a channel in, the main-thread
//! callback queue out.

use std::cell::Cell;
use std::marker::PhantomData;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;

use super::{Raw, Replies, WorkerMessage, deliver};

thread_local! {
    static IN_WORKER: Cell<bool> = const { Cell::new(false) };
}

pub(super) fn is_worker() -> bool {
    IN_WORKER.with(Cell::get)
}

/// The UI side of a running worker thread.
pub(super) struct Handle {
    tx: mpsc::Sender<Vec<u8>>,
    stop: Arc<AtomicBool>,
}

impl Handle {
    pub(super) fn send(&self, bytes: Vec<u8>) {
        // The thread has ended (a panic): the message has nowhere to go.
        let _ = self.tx.send(bytes);
    }

    /// Cooperative: the flag silences replies and stops the loop, and
    /// dropping the sender (with `self`) ends a thread waiting for a message.
    pub(super) fn terminate(self) {
        self.stop.store(true, Ordering::Release);
    }
}

/// The worker side of the channel back to the UI thread.
pub(super) struct Sink {
    id: u64,
    stop: Arc<AtomicBool>,
}

impl Sink {
    pub(super) fn send(&self, bytes: Vec<u8>) {
        self.post(Raw::Reply(bytes));
    }

    pub(super) fn is_terminated(&self) -> bool {
        self.stop.load(Ordering::Acquire)
    }

    fn post(&self, raw: Raw) {
        if self.is_terminated() {
            return;
        }
        let id = self.id;
        rinch_core::dispatch_main_callback(Box::new(move || deliver(id, raw)));
    }
}

pub(super) fn spawn<In: WorkerMessage, Out: WorkerMessage>(
    id: u64,
    handler: fn(In, &Replies<Out>),
) -> Option<Handle> {
    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    let stop = Arc::new(AtomicBool::new(false));
    let sink_stop = stop.clone();
    let started = std::thread::Builder::new()
        .name(format!("rinch-worker-{id}"))
        .spawn(move || {
            IN_WORKER.with(|w| w.set(true));
            let replies = Replies::<Out> {
                sink: Sink {
                    id,
                    stop: sink_stop,
                },
                _out: PhantomData,
            };
            while let Ok(bytes) = rx.recv() {
                if replies.sink.is_terminated() {
                    break;
                }
                let Some(msg) = In::from_bytes(bytes) else {
                    replies
                        .sink
                        .post(Raw::Error("a message could not be decoded".into()));
                    continue;
                };
                if let Err(panic) = catch_unwind(AssertUnwindSafe(|| handler(msg, &replies))) {
                    replies.sink.post(Raw::Failed(format!(
                        "the worker's handler panicked: {}",
                        panic_message(&*panic)
                    )));
                    break;
                }
            }
        });
    if let Err(e) = started {
        let message = format!("the worker thread could not start: {e}");
        rinch_core::dispatch_main_callback(Box::new(move || deliver(id, Raw::Failed(message))));
    }
    Some(Handle { tx, stop })
}

fn panic_message(panic: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = panic.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = panic.downcast_ref::<String>() {
        s.clone()
    } else {
        "(no message)".to_string()
    }
}
