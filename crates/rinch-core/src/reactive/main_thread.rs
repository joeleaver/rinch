//! Cross-thread signal dispatch: the main thread, thread tags, and the main-callback queue.

use std::cell::Cell;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use super::{SIGNAL_STORE, batch};

// ============================================================================
// Cross-Thread Signal Dispatch
// ============================================================================

/// The thread ID of the main (UI) thread, set once at runtime startup.
static MAIN_THREAD_ID: OnceLock<std::thread::ThreadId> = OnceLock::new();

/// Dispatcher function for sending closures to the main thread.
type DispatchFn = fn(Box<dyn FnOnce() + Send>);
static CROSS_THREAD_DISPATCHER: Mutex<Option<DispatchFn>> = Mutex::new(None);

/// Register the current thread as the main (UI) thread.
///
/// Called by the rinch runtime at startup. Must be called from the main thread
/// before any signals are used cross-thread.
pub fn register_main_thread() {
    MAIN_THREAD_ID.get_or_init(|| std::thread::current().id());
}

/// Make the current thread a host of its own: a thread where signals may be
/// written directly, as on the main thread, and that drains the main-thread
/// callbacks meant for it (issue #2778).
///
/// The registered main thread is one per process ([`register_main_thread`]).
/// A test harness runs one app per test thread instead, several at once in one
/// binary (`rinch::testing::TestApp` calls this at its mount), so each test
/// thread is a host of its own:
///
/// - [`is_main_thread`] answers `true` here, so `set`, `update` and effects
///   work whether or not another thread is the registered main thread;
/// - a closure queued here (a [`run_on_main_thread`] that left this thread, a
///   [`dispatch_main_callback`]) waits for this thread's own
///   [`drain_main_callbacks`], not another host's;
/// - a [`Signal::send`](crate::Signal::send) to a signal created here, from
///   any thread, is applied here: at once when sent here, else by this
///   thread's drain. (A signal of a thread that is not a host is written as
///   before #2778: on the registered main thread.)
///
/// Lasts for the thread's life. A real app does not call it: its main thread
/// is the registered one.
pub fn mark_host_thread() {
    QUEUE_THREAD.with(|q| q.host.set(true));
    SIGNAL_STORE.with(|store| store.borrow_mut().claim_tag());
    set_host_tag(current_thread_tag(), true);
}

/// The thread tags of live [host threads](mark_host_thread), one bit each:
/// the threads a write can be queued for. A signal of any other thread (a
/// worker that made signals of its own) is written as before #2778.
static HOST_TAGS: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];

fn set_host_tag(tag: u32, host: bool) {
    if tag == 0 {
        return;
    }
    let (word, bit) = ((tag / 64) as usize, 1u64 << (tag % 64));
    if host {
        HOST_TAGS[word].fetch_or(bit, Ordering::Relaxed);
    } else {
        HOST_TAGS[word].fetch_and(!bit, Ordering::Relaxed);
    }
}

pub(super) fn is_host_tag(tag: u32) -> bool {
    tag != 0 && HOST_TAGS[(tag / 64) as usize].load(Ordering::Relaxed) & (1u64 << (tag % 64)) != 0
}

/// Whether [`mark_host_thread`] made this thread a host.
fn is_host_thread() -> bool {
    QUEUE_THREAD.try_with(|q| q.host.get()).unwrap_or(false)
}

/// Register a function that dispatches closures to the main thread.
///
/// Called by the rinch runtime at startup. The dispatcher should queue the
/// closure for execution on the main thread and wake the event loop.
pub fn set_cross_thread_dispatcher(dispatcher: fn(Box<dyn FnOnce() + Send>)) {
    *CROSS_THREAD_DISPATCHER.lock().unwrap() = Some(dispatcher);
}

/// Register `dispatcher`, but only if no dispatcher is registered yet.
///
/// [`set_cross_thread_dispatcher`] is last-wins, which is what a host owning the
/// process wants: the desktop shell's dispatcher also wakes its winit event
/// loop, and it must win. An embedded context arms cross-thread dispatch through
/// this door instead, so creating one inside a desktop app cannot displace the
/// runtime's waking dispatcher with a queue-only one. Either way both hosts push
/// onto the same [`queue_main_callback`] queue, so whichever dispatcher is
/// installed, every drain site sees every closure.
pub fn set_cross_thread_dispatcher_if_unset(dispatcher: fn(Box<dyn FnOnce() + Send>)) {
    let mut slot = CROSS_THREAD_DISPATCHER.lock().unwrap();
    if slot.is_none() {
        *slot = Some(dispatcher);
    }
}

/// Closures queued from a background thread, waiting to run on the main thread.
///
/// The queue lives here, next to the dispatcher it backs, because *every* host
/// needs the same one: the desktop shell drains it before each paint and on each
/// event-loop wake, the Android loop drains it once per frame, and an embedded
/// `RinchContext` drains it at the top of `update()`. It used to be a `static`
/// private to `shell/rinch_runtime.rs`, which is `desktop`-gated — so an
/// `embed`-only build had nowhere to put a cross-thread closure and
/// [`Signal::send`](crate::Signal::send) panicked in the one mode most likely to
/// want it (issue #172).
///
/// Each closure is queued beside the document and context root whose code
/// queued it and runs in them — see [`queue_main_callback`] — and beside the
/// thread it is for (issue #2778): a thread drains the entries meant for it and
/// the untargeted ones, never another thread's.
static MAIN_QUEUE: Mutex<Vec<QueuedCallback>> = Mutex::new(Vec::new());

/// One closure on [`MAIN_QUEUE`].
struct QueuedCallback {
    /// The thread it is for, by [thread tag](QueueThread::tag); `0` for any
    /// thread that drains the queue.
    target: u32,
    frame: crate::context::CallbackFrame,
    callback: Box<dyn FnOnce() + Send>,
}

impl QueuedCallback {
    /// Whether a drain on the thread tagged `here` runs this.
    fn is_for(&self, here: u32) -> bool {
        self.target == 0 || self.target == here
    }
}

/// What the main-thread queue knows about the current thread.
pub(super) struct QueueThread {
    /// This thread's tag (`0` until its first signal, and when the tags ran
    /// out). Every signal created here carries it in its id
    /// ([`signal_owner`]), so a write from another thread can be queued for
    /// the thread whose store holds the signal.
    pub(super) tag: Cell<u32>,
    /// Made a host of its own by [`mark_host_thread`].
    host: Cell<bool>,
    /// The thread the next [`queue_main_callback`] here is for, set by
    /// [`dispatch_routed`] around the dispatcher it calls (`0`: none).
    route: Cell<u32>,
}

thread_local! {
    pub(super) static QUEUE_THREAD: QueueThread = const {
        QueueThread {
            tag: Cell::new(0),
            host: Cell::new(false),
            route: Cell::new(0),
        }
    };
}

/// This thread's tag, `0` when it has none. Claims nothing.
fn current_thread_tag() -> u32 {
    QUEUE_THREAD.try_with(|q| q.tag.get()).unwrap_or(0)
}

/// A signal id is `tag << TAG_SHIFT | slot`: its thread's tag in the high
/// bits, its slot index in the low ones. The generation keeps all 32 bits, so
/// a freed handle is told from its slot's next signal for 2^32 allocations
/// (with the tag in the generation it was 2^24, and a stale handle revived
/// after 16 777 215 — review of #2780).
pub(super) const TAG_SHIFT: u32 = 24;
/// The slot index part of a signal id: at most 2^24 signals alive at once on
/// one thread.
pub(super) const SLOT_MASK: u32 = (1 << TAG_SHIFT) - 1;
/// The most thread tags alive at once (tag `0` is "none").
const MAX_THREAD_TAG: u32 = u32::MAX >> TAG_SHIFT;

/// The tag of the thread whose store holds the signal with this id, `0` for
/// none known.
pub(crate) fn signal_owner(id: u32) -> u32 {
    id >> TAG_SHIFT
}

/// Thread tags not in use, each with the generation counter to resume from.
pub(super) struct TagPool {
    /// The next never-used tag.
    fresh: u32,
    /// Tags released by threads that ended, oldest first, with where their
    /// generation counter stopped: the next thread to take one continues
    /// from there, so a handle to an ended thread's signal never names a
    /// signal of the new one.
    free: VecDeque<(u32, u32)>,
}

static TAG_POOL: Mutex<TagPool> = Mutex::new(TagPool {
    fresh: 1,
    free: VecDeque::new(),
});

/// Take a thread tag and the counter to start from: `(0, 1)` when every tag is
/// in use (that thread's signals then name no thread, and are written as
/// before #2778).
pub(super) fn claim_thread_tag() -> (u32, u32) {
    TAG_POOL.lock().unwrap_or_else(|e| e.into_inner()).claim()
}

impl TagPool {
    /// See [`claim_thread_tag`].
    fn claim(&mut self) -> (u32, u32) {
        if let Some(reused) = self.free.pop_front() {
            return reused;
        }
        if self.fresh <= MAX_THREAD_TAG {
            let tag = self.fresh;
            self.fresh += 1;
            return (tag, 1);
        }
        (0, 1)
    }

    /// See [`release_thread_tag`].
    fn release(&mut self, tag: u32, counter: u32) {
        if tag != 0 {
            self.free.push_back((tag, counter));
        }
    }
}

/// Give a thread's tag back when its signal store is dropped (the thread is
/// ending). Entries still queued for it are drained by the next thread that
/// takes the tag; their signals are gone, so their writes find nothing.
pub(super) fn release_thread_tag(tag: u32, counter: u32) {
    set_host_tag(tag, false);
    TAG_POOL
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .release(tag, counter);
}

/// Dispatch `f` through `dispatch`, for the thread tagged `owner` (`0`: as
/// unrouted). The tag reaches [`queue_main_callback`] through a thread-local
/// set around the call, so a dispatcher keeps its one-argument shape: every
/// host's dispatcher queues on the thread that calls it.
fn dispatch_routed(
    owner: u32,
    f: Box<dyn FnOnce() + Send>,
    dispatch: fn(Box<dyn FnOnce() + Send>),
) {
    if owner == 0 {
        dispatch(f);
        return;
    }
    let previous = QUEUE_THREAD.try_with(|q| q.route.replace(owner)).ok();
    dispatch(f);
    if let Some(previous) = previous {
        let _ = QUEUE_THREAD.try_with(|q| q.route.set(previous));
    }
}

/// Push `f` onto the shared main-thread queue.
///
/// Returns `true` if the queue was empty before the push. No host keys its wake
/// on it any more: a desktop paint can drain the wake it asked for without
/// running the queue (issue #1540), so the shell asks for a coalesced
/// `ReRender` on every push instead. A host with nothing to wake (an embedded
/// context, whose game loop is already turning) ignores it.
///
/// `f` will run under the document current **here**, on the thread that queues
/// it (issue #963): the document whose code queued it on the main thread, and
/// none for a closure queued from a worker thread, which is no document's code.
/// The drain runs between events, or inside another embedded context's
/// `update()`, so without this a closure queued by one document's handler ran
/// under no document or a borrowed one. It also runs in the **context root**
/// current here (issue #1032), so a closure queued by an embedded
/// `RinchContext`'s code resolves that context's stores, as an effect does
/// (#136); one queued from a worker thread runs at the thread-global root.
///
/// Which thread drains it (issue #2778): a [`Signal::send`](crate::Signal::send)
/// write to a signal a [host thread](mark_host_thread) created is for that
/// thread; any other closure queued on a host thread is for that thread; the rest (a
/// worker's [`run_on_main_thread`], everything in an app with one main thread)
/// are for whichever thread drains first, as before.
pub fn queue_main_callback(f: Box<dyn FnOnce() + Send>) -> bool {
    let frame = crate::context::CallbackFrame::current();
    let target = QUEUE_THREAD
        .try_with(|q| match q.route.replace(0) {
            0 if q.host.get() => q.tag.get(),
            routed => routed,
        })
        .unwrap_or(0);
    let mut queue = MAIN_QUEUE.lock().unwrap();
    let was_empty = queue.is_empty();
    queue.push(QueuedCallback {
        target,
        frame,
        callback: f,
    });
    was_empty
}

/// Run every queued main-thread callback meant for this thread. Call from the
/// main thread, or a [host thread](mark_host_thread), only: called on any other
/// thread it runs nothing, leaves the queue for the threads it is for, and logs
/// one warning (#2637).
///
/// The queue is process-global, and every host on this thread drains the same
/// entries, so a host that drains it runs the work queued against every host on
/// the thread. That is correct rather than merely tolerable: the payload is a
/// `Send` closure that writes its own signals, and signals are thread-local —
/// not per-document — so it does the same thing whoever on the thread runs it.
/// The document it ultimately touches is still repainted by its own host,
/// because a signal change notifies every subscriber (issue #134).
///
/// What it does not run is another thread's (issue #2778): a write to a signal
/// another thread created, or a closure another host thread queued, waits for
/// that thread's drain. Signal ids alias across thread-local stores, so run
/// here such a write would land in this thread's signal of the same id.
pub fn drain_main_callbacks() {
    // Off the main thread the queue is not this thread's to run (#2637): an
    // embedded context updated on a worker took the main thread's work — a
    // fired timer among it — and ran it where its parked callback was not, so
    // the timer was lost and a `sleep` loop on it stopped for good. It stays
    // queued for the main thread's own drain. A host thread (#2778: one per
    // `TestApp`) answers `is_main_thread` and drains the entries meant for it.
    if !is_main_thread() {
        static SAID: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        if !SAID.swap(true, std::sync::atomic::Ordering::Relaxed) {
            tracing::warn!(
                "rinch: the main-thread queue was drained off the main thread (an embedded \
                 RinchContext updated on a worker?); it is left for the main thread (#2637)"
            );
        }
        return;
    }
    let here = current_thread_tag();
    let callbacks: Vec<QueuedCallback> = MAIN_QUEUE
        .lock()
        .unwrap()
        .extract_if(.., |entry| entry.is_for(here))
        .collect();
    // One transaction per callback, not one for the whole drain: callbacks are
    // queued independently (a `Signal::send`, a timer, a parked continuation),
    // and a later one may rely on an earlier one's effects having run. Each runs
    // under the document that queued it (issue #963), in its context root
    // (issue #1032).
    for QueuedCallback {
        frame, callback, ..
    } in callbacks
    {
        let _frame = frame.enter();
        batch(callback);
    }
}

/// Whether any main-thread callback is queued that
/// [`drain_main_callbacks`] would run on this thread.
///
/// For a host that coalesces its wakes: a callback queued while the host was
/// between emptying this queue and re-arming its own wake signal can have had
/// its wake folded into the one being served (issue #988), so the host asks
/// this after re-arming and wakes itself again.
pub fn main_callbacks_pending() -> bool {
    let here = current_thread_tag();
    MAIN_QUEUE
        .lock()
        .unwrap()
        .iter()
        .any(|entry| entry.is_for(here))
}

/// Drop every queued main-thread callback [`drain_main_callbacks`] would run
/// on this thread, without running it (another host thread's stay queued).
///
/// For host shutdown only: a queued closure typically captures app state that is
/// about to be torn down, so running it there would be worse than losing it.
///
/// The closures are dropped **after** the queue's lock is released (issue
/// #1061): a closure's captures may queue main-thread work from their own
/// `Drop` — a render surface's lease releasing itself, a user type calling
/// `Signal::send` — and the lock is not re-entrant, so dropping under it
/// deadlocked shutdown. What such a drop queues is pending work like any other,
/// so the queue is emptied again until it stays empty, up to
/// `MAX_CLEAR_ROUNDS` rounds (a `Drop` that queues a closure whose `Drop`
/// queues another, without end, would otherwise never let shutdown finish).
pub fn clear_main_callbacks() {
    const MAX_CLEAR_ROUNDS: usize = 16;
    for _ in 0..MAX_CLEAR_ROUNDS {
        // The guard is a temporary of this statement, released before the drop.
        let here = current_thread_tag();
        let dropped: Vec<QueuedCallback> = MAIN_QUEUE
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .extract_if(.., |entry| entry.is_for(here))
            .collect();
        if dropped.is_empty() {
            return;
        }
        drop(dropped);
    }
    tracing::warn!(
        "clear_main_callbacks: queued closures kept queueing more from their Drop \
         after {MAX_CLEAR_ROUNDS} rounds; leaving the rest queued"
    );
}

/// Check if the current thread is the main (UI) thread.
///
/// Returns `true` if `register_main_thread()` hasn't been called yet
/// (backwards compatibility for tests and non-rinch usage), and on a
/// [host thread](mark_host_thread).
pub fn is_main_thread() -> bool {
    match MAIN_THREAD_ID.get() {
        Some(id) => std::thread::current().id() == *id || is_host_thread(),
        None => true,
    }
}

/// Which thread a cross-thread write ([`Signal::send`](crate::Signal::send),
/// [`Signal::update_send`](crate::Signal::update_send)) is being made on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum WriteThread {
    /// The registered main thread, or a [host thread](mark_host_thread): apply
    /// the write here (if the signal is this thread's, see [`route_write`]).
    Main,
    /// Another thread, with a main thread registered: dispatch it.
    Other,
    /// No host has registered a main thread yet (issue #1541).
    Unregistered,
}

/// Classify the current thread for a cross-thread write. Costs what
/// [`is_main_thread`] costs; only the [`WriteThread::Unregistered`] answer
/// leaves the caller anything more to decide.
pub(crate) fn write_thread() -> WriteThread {
    match MAIN_THREAD_ID.get() {
        Some(id) if std::thread::current().id() == *id => WriteThread::Main,
        Some(_) if is_host_thread() => WriteThread::Main,
        Some(_) => WriteThread::Other,
        None if is_host_thread() => WriteThread::Main,
        None => WriteThread::Unregistered,
    }
}

/// Where a cross-thread write ([`Signal::send`](crate::Signal::send),
/// [`Signal::update_send`](crate::Signal::update_send)) to the signal with
/// id `id` goes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum WriteRoute {
    /// Apply it here, now.
    Here,
    /// Queue it through the registered dispatcher (which must exist), for the
    /// thread tagged `owner` (`0`: whichever main thread drains).
    Dispatch { owner: u32 },
    /// Queue it through [`dispatch_main_callback`] (no dispatcher needed),
    /// for the thread tagged `owner`; with `owner == 0` it is applied only if
    /// drained on the registered main thread (issue #1541).
    Queue { owner: u32 },
}

/// Decide [`WriteRoute`] for a write made on this thread (issue #2778).
///
/// A signal names the thread that created it ([`signal_owner`]). When that is
/// a [host thread](mark_host_thread), the write is applied here if this is
/// that thread, and is otherwise queued for its drain. Any other signal (an
/// app's main thread's, a worker's own) is routed as before #2778: to the
/// registered main thread. Nothing is queued for a thread that never drains;
/// a main thread's drain of a write to a worker's signal finds no such signal
/// in its own store (the id names the worker's tag) and drops it with the
/// warning that it was made on a thread that hosts no app (issue #2889),
/// where it used to write the main thread's signal of
/// the same slot. On a main thread with no host threads in the process this
/// costs one relaxed atomic load more than before.
pub(crate) fn route_write(id: u32) -> WriteRoute {
    let tag = signal_owner(id);
    let owner = if is_host_tag(tag) { tag } else { 0 };
    let ours = owner != 0 && owner == current_thread_tag();
    match write_thread() {
        WriteThread::Main if owner == 0 || ours => WriteRoute::Here,
        WriteThread::Main => WriteRoute::Queue { owner },
        WriteThread::Other => WriteRoute::Dispatch { owner },
        WriteThread::Unregistered if ours => WriteRoute::Here,
        WriteThread::Unregistered if owner == 0 && unregistered_write_is_local() => {
            WriteRoute::Here
        }
        WriteThread::Unregistered => WriteRoute::Queue { owner },
    }
}

/// With no main thread registered, whether a cross-thread write made on this
/// thread is this thread's to apply (issue #1541).
///
/// A thread that has never allocated a signal cannot own the one being
/// written — signals live in a thread-local store — so it is a worker
/// writing ahead of the host (a thread spawned in `main()` before
/// `App::run`), and the write is queued for the main thread instead of being
/// applied to this thread's empty store and dropped. A thread that has
/// allocated signals keeps the old behaviour (apply here): the main thread
/// of a test or of `rinch-web`, which registers no main thread at all.
pub(crate) fn unregistered_write_is_local() -> bool {
    SIGNAL_STORE.with(|store| store.borrow().has_allocated())
}

/// Dispatch a closure to the main thread via the registered dispatcher.
///
/// Panics if no dispatcher has been registered (i.e., rinch runtime not
/// initialized). Reaching this at all means [`register_main_thread`] was called,
/// which only a host does — so a host that registers the main thread without
/// also registering a dispatcher is strictly worse off than one that registers
/// neither, where [`is_main_thread`] answers `true` and `send()` degrades to a
/// direct `set()`. Registering both is the contract; the panic is what catches a
/// host that forgot (issue #172).
pub(crate) fn dispatch_to_main_thread(f: Box<dyn FnOnce() + Send>) {
    dispatch_to_main_thread_for(0, f);
}

/// [`dispatch_to_main_thread`], with the closure queued for the thread tagged
/// `owner` (issue #2778; `0` for whichever main thread drains).
pub(crate) fn dispatch_to_main_thread_for(owner: u32, f: Box<dyn FnOnce() + Send>) {
    // Copied out so the dispatcher runs with the lock released: a dispatcher
    // may drop `f`, and a `Drop` in `f`'s captures may dispatch again from this
    // same thread (issue #1061), as in `dispatch_main_callback`.
    let dispatcher = *CROSS_THREAD_DISPATCHER.lock().unwrap();
    if let Some(dispatch) = dispatcher {
        dispatch_routed(owner, f, dispatch);
    } else {
        panic!(
            "Signal::send() called but no cross-thread dispatcher is registered. \
             Ensure the rinch runtime is initialized before using send() from background threads."
        );
    }
}

/// Queue `f` for the main thread through the registered dispatcher, from any
/// thread, without panicking for a missing dispatcher.
///
/// For a caller that must not panic and must not run `f` inline — a `Drop`,
/// which may run on any thread, inside a borrow `f` needs, or at thread exit.
/// Where a dispatcher is registered it gets `f` exactly as a `Signal::send`
/// would, so a host that wakes its loop for a closure queued onto an empty
/// queue (the desktop shell, Android) wakes for this one too. Where none is,
/// `f` is queued plainly: there is no host to wake, and whatever drains the
/// queue will run it.
///
/// Why not [`queue_main_callback`] from such a caller: a closure pushed past
/// the dispatcher gets no wake of its own and waits for the next sender's. A
/// host whose dispatcher wakes only for a push onto an empty queue (the
/// desktop shell's rule until #1540) strands it for good: the next sender
/// finds the queue non-empty and asks for no wake either (issue #1035).
///
/// The dispatcher is called with its lock released, so a dispatcher that drops
/// something whose `Drop` dispatches in turn does not deadlock, and a poisoned
/// lock is read through rather than propagated. That is this function's own
/// guarantee, not its callees': the queue itself ([`queue_main_callback`]) and
/// the desktop dispatcher still `unwrap` their locks, so either would panic if
/// its lock were poisoned (nothing panics while holding them).
pub fn dispatch_main_callback(f: Box<dyn FnOnce() + Send>) {
    dispatch_main_callback_for(0, f);
}

/// [`dispatch_main_callback`], with the closure queued for the thread tagged
/// `owner` (issue #2778; `0` as unrouted).
pub(crate) fn dispatch_main_callback_for(owner: u32, f: Box<dyn FnOnce() + Send>) {
    let dispatcher = *CROSS_THREAD_DISPATCHER
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    match dispatcher {
        Some(dispatch) => dispatch_routed(owner, f, dispatch),
        None => {
            let previous = QUEUE_THREAD.try_with(|q| q.route.replace(owner)).ok();
            queue_main_callback(f);
            if let Some(previous) = previous {
                let _ = QUEUE_THREAD.try_with(|q| q.route.set(previous));
            }
        }
    }
}

/// Run a closure on the main (UI) thread.
///
/// If called from the main thread, runs `f` immediately. Otherwise dispatches it
/// via the registered cross-thread dispatcher (installed by the rinch runtime) to
/// run on the next event-loop wake. Panics only if called from a background thread
/// with no dispatcher registered.
///
/// This is the transport half of the main-thread callback machinery; see
/// [`crate::main_thread`] for parking a `!Send` continuation and resuming it here.
pub fn run_on_main_thread(f: impl FnOnce() + Send + 'static) {
    if is_main_thread() {
        f();
    } else {
        dispatch_to_main_thread(Box::new(f));
    }
}

#[cfg(test)]
mod thread_tag_tests;
