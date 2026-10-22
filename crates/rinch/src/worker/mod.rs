//! Background workers that reply on the UI thread (issue #1772).
//!
//! A [`Worker`] runs a handler — a plain `fn(In, &Replies<Out>)` — off the UI
//! thread, once per message, in the order the messages were sent. What the
//! handler [`send`](Replies::send)s comes back on the UI thread, to an
//! [`on_event`](Worker::on_event) callback or to an awaited
//! [`next`](Worker::next):
//!
//! ```ignore
//! use rinch::worker::{Replies, Worker, WorkerEvent};
//!
//! fn count_words(text: String, replies: &Replies<String>) {
//!     replies.send(text.split_whitespace().count().to_string());
//! }
//!
//! let worker = Worker::<String, String>::spawn(count_words);
//! worker.on_event(move |event| {
//!     if let WorkerEvent::Message(n) = event { words.set(n) }
//! });
//! worker.send(document_text);
//! ```
//!
//! - **Desktop, Android, embed**: the worker is a thread of its own.
//!   [`terminate`](Worker::terminate) is *cooperative*: it drops the messages
//!   not yet handled and silences the replies, and the thread ends when the
//!   running handler returns. A long handler should poll
//!   [`Replies::is_terminated`] and return early (an interpreter's interrupt
//!   hook is the place for it).
//! - **Web**: the worker is a Web Worker running a second instance of the
//!   app's own wasm module, so no extra build step is needed; `terminate` stops
//!   it at once, mid-handler. That instance runs the app's start function too:
//!   `rinch_web::mount*` return without mounting there (there is no
//!   `document`), and code an app runs before mounting can ask [`is_worker`].
//!   It needs the wasm-bindgen glue built for `--target web` (trunk's default)
//!   and a Content-Security-Policy that allows `blob:` workers.
//!
//! A handler is synchronous: nothing in a worker runs rinch tasks or timers,
//! so [`spawn_local`](rinch_core::spawn_local) and
//! [`set_timeout`](rinch_core::set_timeout) panic inside one, and a worker
//! cannot [`spawn`](Worker::spawn) workers.
//!
//! Messages cross as bytes, through [`WorkerMessage`], on every platform, so a
//! codec that loses something loses it on the desktop as well as on the web.
//! Each worker has its own state: a `thread_local!` in the handler's module is
//! per worker (a thread on the desktop, a separate wasm instance on the web).
//! On the web a worker shares nothing else with the UI either; on the desktop
//! it shares the process's statics, so a handler should not rely on them.
//!
//! A worker spawned while a component renders is terminated when that
//! component unmounts, and dropping the [`Worker`] terminates it too.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::marker::PhantomData;
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Poll, Waker};

use rinch_core::CallbackFrame;
use rinch_core::reactive::{Owner, current_owner, unowned};

#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(not(target_arch = "wasm32"))]
use native as backend;

#[cfg(target_arch = "wasm32")]
mod web;
#[cfg(target_arch = "wasm32")]
use web as backend;

#[cfg(target_arch = "wasm32")]
#[doc(hidden)]
pub use web::__rinch_worker_run;

/// How a message crosses to or from a worker: as bytes.
///
/// Implemented for [`String`] (UTF-8) and `Vec<u8>`. An app with structured
/// messages implements it with the codec it already uses.
pub trait WorkerMessage: Sized + 'static {
    /// Encode the message.
    fn into_bytes(self) -> Vec<u8>;
    /// Decode a message; `None` when `bytes` is not one. A message that does
    /// not decode is reported as a [`WorkerEvent::Error`].
    fn from_bytes(bytes: Vec<u8>) -> Option<Self>;
}

impl WorkerMessage for Vec<u8> {
    fn into_bytes(self) -> Vec<u8> {
        self
    }
    fn from_bytes(bytes: Vec<u8>) -> Option<Self> {
        Some(bytes)
    }
}

impl WorkerMessage for String {
    fn into_bytes(self) -> Vec<u8> {
        String::into_bytes(self)
    }
    fn from_bytes(bytes: Vec<u8>) -> Option<Self> {
        String::from_utf8(bytes).ok()
    }
}

/// What a worker reports to the UI thread.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum WorkerEvent<Out> {
    /// A reply the handler sent.
    Message(Out),
    /// Something went wrong. After a handler panic, or a worker that could
    /// not start, the worker has ended: no event follows. A message that did
    /// not decode is reported here too, and the worker goes on.
    Error(String),
}

/// The worker side: where a handler sends its replies.
pub struct Replies<Out> {
    sink: backend::Sink,
    _out: PhantomData<fn(Out)>,
}

impl<Out: WorkerMessage> Replies<Out> {
    /// Send a reply to the UI thread. Dropped once the worker is terminated.
    pub fn send(&self, msg: Out) {
        self.sink.send(msg.into_bytes());
    }

    /// Whether the UI thread has terminated this worker. On the desktop a
    /// handler that may run long should poll it and return early; on the web a
    /// terminated worker has already stopped, so this answers `false`.
    pub fn is_terminated(&self) -> bool {
        self.sink.is_terminated()
    }
}

/// Whether this code is running inside a worker: a [`Worker`]'s thread on the
/// desktop, a worker's wasm instance on the web.
///
/// On the web the app's start function runs again in every worker; code there
/// that needs a page (anything before `rinch_web::mount`) can return early when
/// this is `true`.
pub fn is_worker() -> bool {
    backend::is_worker()
}

/// What a backend hands the UI thread.
pub(crate) enum Raw {
    Reply(Vec<u8>),
    /// A recoverable error: the worker goes on.
    Error(String),
    /// The worker has ended (a handler panic, a failed start).
    Failed(String),
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// A worker's type-erased receiver, on the UI thread.
type Receiver = Rc<dyn Fn(Raw)>;

thread_local! {
    /// Live workers on the UI thread, by id.
    static WORKERS: RefCell<HashMap<u64, Receiver>> = RefCell::new(HashMap::new());
}

/// Hand a backend event to worker `id`. Called on the UI thread; a worker that
/// has been terminated or dropped is no longer listed and drops it.
pub(crate) fn deliver(id: u64, raw: Raw) {
    let receiver = WORKERS.with(|w| w.borrow().get(&id).cloned());
    if let Some(receiver) = receiver {
        receiver(raw);
    }
}

/// Unlist worker `id` and stop its backend. Shared by [`Inner::end`] and
/// `Worker`'s `Drop`, which cannot name `Out: WorkerMessage`.
fn end(id: u64, handle: &RefCell<Option<backend::Handle>>) {
    WORKERS.with(|w| w.borrow_mut().remove(&id));
    if let Some(handle) = handle.borrow_mut().take() {
        handle.terminate();
    }
}

/// A callback registered with [`Worker::on_event`], with what it runs under.
struct Slot<Out> {
    owner: Option<Owner>,
    frame: CallbackFrame,
    f: Rc<dyn Fn(WorkerEvent<Out>)>,
}

struct Inner<Out> {
    id: u64,
    handle: RefCell<Option<backend::Handle>>,
    events: RefCell<VecDeque<WorkerEvent<Out>>>,
    /// Every task waiting in [`Worker::next`]; all are woken.
    wakers: RefCell<Vec<Waker>>,
    callback: RefCell<Option<Slot<Out>>>,
    /// The callback's component has unmounted: events are dropped, not
    /// queued for `next`, until `on_event` is called again.
    discarding: Cell<bool>,
    ended: Cell<bool>,
}

impl<Out: WorkerMessage> Inner<Out> {
    fn receive(&self, raw: Raw) {
        if self.ended.get() {
            return;
        }
        let event = match raw {
            Raw::Reply(bytes) => match Out::from_bytes(bytes) {
                Some(msg) => WorkerEvent::Message(msg),
                None => WorkerEvent::Error("a reply could not be decoded".into()),
            },
            Raw::Error(e) => WorkerEvent::Error(e),
            Raw::Failed(e) => {
                self.end();
                WorkerEvent::Error(e)
            }
        };
        if self.discarding.get() {
            return;
        }
        let callback = self
            .callback
            .borrow()
            .as_ref()
            .map(|s| (s.owner.clone(), s.frame, s.f.clone()));
        match callback {
            Some((owner, frame, f)) => {
                if owner.as_ref().is_some_and(|o| !o.is_alive()) {
                    // The receiver is gone with its component: drop this event
                    // and the ones after it rather than piling them up for a
                    // `next` nobody may call.
                    self.callback.borrow_mut().take();
                    self.discarding.set(true);
                    return;
                }
                let _frame = frame.enter();
                match owner {
                    Some(owner) => rinch_core::batch(|| owner.run(|| f(event))),
                    None => rinch_core::batch(|| unowned(|| f(event))),
                }
            }
            None => {
                self.events.borrow_mut().push_back(event);
                self.wake();
            }
        }
    }

    /// Stop receiving: unlist the worker and stop its backend. Events already
    /// queued stay readable; nothing new arrives.
    fn end(&self) {
        self.ended.set(true);
        end(self.id, &self.handle);
        self.wake();
    }

    fn wake(&self) {
        let wakers = std::mem::take(&mut *self.wakers.borrow_mut());
        for waker in wakers {
            waker.wake();
        }
    }
}

/// A background worker, held on the UI thread. See the [module docs](self).
///
/// Not `Send`. Dropping it terminates the worker.
pub struct Worker<In, Out> {
    inner: Rc<Inner<Out>>,
    _in: PhantomData<fn(In)>,
}

impl<In: WorkerMessage, Out: WorkerMessage> Worker<In, Out> {
    /// Start a worker that calls `handler` once per message.
    ///
    /// `handler` is a function, not a closure: on the web it runs in another
    /// wasm instance, where nothing it captured would exist. State it keeps
    /// between messages goes in a `thread_local!`.
    ///
    /// Spawned while a component renders, the worker is terminated when the
    /// component unmounts.
    ///
    /// Called inside a worker (see [`is_worker`]) it starts nothing: the
    /// `Worker` it returns has ended, and its [`next`](Self::next) yields one
    /// [`WorkerEvent::Error`] saying why. On the web this is what stops an app
    /// whose start function spawns a worker from spawning one per instance.
    pub fn spawn(handler: fn(In, &Replies<Out>)) -> Self {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let inner = Rc::new(Inner {
            id,
            handle: RefCell::new(None),
            events: RefCell::new(VecDeque::new()),
            wakers: RefCell::new(Vec::new()),
            callback: RefCell::new(None),
            discarding: Cell::new(false),
            ended: Cell::new(false),
        });
        if is_worker() {
            // On the web the app's start function runs again in every worker
            // instance, so a worker it spawns would spawn itself without end.
            // Refused on both backends: a worker spawns no worker.
            const REFUSED: &str = "Worker::spawn was called inside a worker; a worker cannot spawn \
                 workers (on the web the app's start function runs again in each worker: \
                 check rinch::worker::is_worker() there)";
            tracing::warn!("{REFUSED}");
            inner
                .events
                .borrow_mut()
                .push_back(WorkerEvent::Error(REFUSED.into()));
            inner.ended.set(true);
            return Self {
                inner,
                _in: PhantomData,
            };
        }
        let weak: Weak<Inner<Out>> = Rc::downgrade(&inner);
        let receiver: Receiver = Rc::new(move |raw| {
            if let Some(inner) = weak.upgrade() {
                inner.receive(raw);
            }
        });
        WORKERS.with(|w| w.borrow_mut().insert(id, receiver));
        *inner.handle.borrow_mut() = backend::spawn::<In, Out>(id, handler);

        let weak = Rc::downgrade(&inner);
        rinch_core::reactive::on_cleanup(move || {
            if let Some(inner) = weak.upgrade() {
                inner.end();
            }
        });
        Self {
            inner,
            _in: PhantomData,
        }
    }

    /// Send a message. Messages are handled one at a time, in order. Ignored
    /// once the worker has ended.
    pub fn send(&self, msg: In) {
        if let Some(handle) = self.inner.handle.borrow().as_ref() {
            handle.send(msg.into_bytes());
        }
    }

    /// Receive the worker's events with a callback, on the UI thread, one
    /// [`batch`](rinch_core::batch) each. Replaces an earlier callback.
    /// Events that arrive while a callback is set do not reach
    /// [`next`](Self::next); events that arrived before it stay there.
    ///
    /// Registered while a component renders, the callback stops being called
    /// when that component unmounts, and the worker's events from then on are
    /// dropped (not kept for `next`) until `on_event` is called again.
    pub fn on_event(&self, f: impl Fn(WorkerEvent<Out>) + 'static) {
        self.inner.discarding.set(false);
        *self.inner.callback.borrow_mut() = Some(Slot {
            owner: current_owner(),
            frame: CallbackFrame::current(),
            f: Rc::new(f),
        });
    }

    /// The next event, awaited — inside [`spawn_local`](rinch_core::spawn_local)
    /// say. `None` once the worker has ended (terminated, dropped with its
    /// component, or failed) and every event it sent before has been read.
    /// Several tasks may wait at once: each event goes to one of them, and all
    /// see the end.
    pub async fn next(&self) -> Option<WorkerEvent<Out>> {
        std::future::poll_fn(|cx| {
            if let Some(event) = self.inner.events.borrow_mut().pop_front() {
                return Poll::Ready(Some(event));
            }
            if self.inner.ended.get() {
                return Poll::Ready(None);
            }
            let mut wakers = self.inner.wakers.borrow_mut();
            if !wakers.iter().any(|w| w.will_wake(cx.waker())) {
                wakers.push(cx.waker().clone());
            }
            Poll::Pending
        })
        .await
    }

    /// Stop the worker: on the web at once, on the desktop when the running
    /// handler returns (see [`Replies::is_terminated`]). Messages not yet
    /// handled are dropped, and no event arrives afterwards; a pending
    /// [`next`](Self::next) resolves to `None` once the events already
    /// received are read.
    pub fn terminate(&self) {
        self.inner.end();
    }

    /// Whether the worker has ended (terminated, or failed).
    pub fn is_ended(&self) -> bool {
        self.inner.ended.get()
    }
}

impl<In, Out> Drop for Worker<In, Out> {
    fn drop(&mut self) {
        end(self.inner.id, &self.inner.handle);
    }
}
