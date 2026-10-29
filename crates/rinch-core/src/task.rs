//! Futures on the UI thread: [`spawn_local`] and [`sleep`] (issue #1767).
//!
//! rinch's asynchronous APIs are callbacks — `set_timeout`, `rinch_http::fetch`,
//! `rinch_ws` — and each runs its callback on the main (UI) thread. This module
//! is the same guarantee for a `Future`: [`spawn_local`] runs a `!Send` future
//! on the thread that spawned it, so it may capture `Rc`s and set
//! [`Signal`](crate::Signal)s directly, on the desktop and in the browser alike.
//!
//! ```ignore
//! spawn_local(async move {
//!     status.set("signing in…".into());
//!     let me = rinch_http::fetch_async(Request::post(login_url)).await;
//!     rinch_core::task::sleep(Duration::from_millis(300)).await;
//!     status.set(format!("{me:?}"));
//! });
//! ```
//!
//! # How a task is driven
//!
//! A task lives in a main-thread-local table, keyed by an id; only the id ever
//! crosses a thread. Its [`Waker`] may be woken from any thread (a worker that
//! finished a request, the timer thread), and asks for **one** poll however many
//! times it is woken before that poll runs:
//!
//! - where a backend installed a scheduler ([`set_task_scheduler`]), that is
//!   called — `rinch-web` queues a microtask;
//! - otherwise the poll is queued with
//!   [`dispatch_main_callback`](crate::dispatch_main_callback), the queue every
//!   host already drains: the desktop shell and the Android loop wake for it, an
//!   embedded `RinchContext` drains it at the top of `update()`. No thread is
//!   started.
//!
//! A task is never polled inside [`spawn_local`]: its first poll is scheduled
//! like any other, so its code does not run inside the caller's render or
//! borrow. Each poll is one [`batch`](crate::batch), under the document and
//! context root the task was spawned in (as a timer is, #963/#1032).
//!
//! # Lifetime
//!
//! A task spawned **during a render** belongs to the component being rendered:
//! when that component unmounts its future is **dropped**, unpolled — and with
//! it everything the future holds (a `WsStream`'s socket, a pending [`sleep`]'s
//! timer). That is the rule [`set_timeout`](crate::set_timeout) and a parked
//! continuation follow, for the same reason: the future captured the component's
//! signals, which are freed with it (issue #141).
//!
//! Spawn outside any render — or under [`unowned`](crate::reactive::unowned) —
//! for a task that must outlive its component (a save on close):
//!
//! ```ignore
//! rinch_core::reactive::unowned(|| spawn_local(async move { flush().await }));
//! ```

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Wake, Waker};
use std::time::Duration;

use crate::context::CallbackFrame;
use crate::reactive::{Owner, current_owner, unowned};
use crate::timer::{TimeoutHandle, clear_timeout, set_timeout};

thread_local! {
    /// Set on a `rinch::worker` thread (desktop) or in a worker's wasm instance
    /// (web), where nothing drives tasks or timers (issue #1772).
    static IN_WORKER: Cell<bool> = const { Cell::new(false) };
}

/// Mark this thread as running `rinch::worker` handlers: [`spawn_local`] and
/// [`set_timeout`] / [`set_interval`](crate::timer::set_interval) panic here from now on instead of silently never running.
#[doc(hidden)]
pub fn __enter_worker_context() {
    IN_WORKER.with(|w| w.set(true));
}

/// Panic when called inside a `rinch::worker` handler. A handler is
/// synchronous: no loop polls a task or fires a timer in a worker, so a task
/// or timer armed there would never run.
pub(crate) fn refuse_in_worker(what: &str) {
    if IN_WORKER.with(Cell::get) {
        panic!(
            "{what} cannot be used inside a rinch::worker handler: handlers are \
             synchronous, and nothing in a worker runs tasks or timers"
        );
    }
}

/// Schedules [`poll_task`] for a task id **on the main thread**, later. Called
/// from whatever thread woke the task. Installed with [`set_task_scheduler`].
type TaskScheduler = fn(id: u64);

static SCHEDULER: Mutex<Option<TaskScheduler>> = Mutex::new(None);

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// A spawned future and what it runs under.
struct Task {
    future: Pin<Box<dyn Future<Output = ()>>>,
    /// The scope rendering at spawn, if any; `None` is app lifetime.
    owner: Option<Owner>,
    /// The document and context root at spawn, re-entered around each poll.
    frame: CallbackFrame,
    waker: Arc<TaskWaker>,
}

/// A task's entry in [`TASKS`].
enum Slot {
    /// Waiting for a poll.
    Idle(Task),
    /// Taken out and being polled right now. What happens to it meanwhile is
    /// recorded here and applied when the poll returns.
    Running {
        /// [`TaskHandle::abort`] or its owner's unmount came during the poll.
        aborted: bool,
        /// A queued poll ran (a nested drain) while this one was running.
        woken: bool,
    },
}

/// The task table. Its `Drop` — this thread ending with tasks still pending —
/// forgets them rather than dropping them: a future's captures are user code
/// (a `WsHandle`, a `Sleep`, an editor handle) whose `Drop` reaches for other
/// thread-locals, which may already be gone at that point, and a panic in a TLS
/// destructor aborts the process. The thread is ending; nothing is lost.
#[derive(Default)]
struct TaskTable(HashMap<u64, Slot>);

impl Drop for TaskTable {
    fn drop(&mut self) {
        for (_, slot) in self.0.drain() {
            std::mem::forget(slot);
        }
    }
}

impl std::ops::Deref for TaskTable {
    type Target = HashMap<u64, Slot>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for TaskTable {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

thread_local! {
    /// Every live task of this thread, by id. Thread-local so the futures need
    /// not be `Send`.
    static TASKS: RefCell<TaskTable> = RefCell::new(TaskTable::default());

    /// The live tasks of each owning scope, keyed by [`Owner::key`], with the
    /// owner itself kept beside them so the key's address cannot be reused while
    /// the entry exists. One cleanup per owner — registered by its first spawn,
    /// removing the entry — rather than one per spawn, so a component that
    /// spawns on every keystroke does not grow its cleanup list.
    static OWNER_TASKS: RefCell<HashMap<usize, (Owner, HashSet<u64>)>> =
        RefCell::new(HashMap::new());
}

/// The [`Waker`] of one task: `Send + Sync`, holding nothing but the id.
struct TaskWaker {
    id: u64,
    /// A poll is scheduled and has not started yet. Cleared just before each
    /// poll, so a wake during the poll schedules the next one.
    scheduled: AtomicBool,
}

impl Wake for TaskWaker {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        if !self.scheduled.swap(true, Ordering::AcqRel) {
            schedule(self.id);
        }
    }
}

fn schedule(id: u64) {
    let scheduler = *SCHEDULER.lock().unwrap_or_else(|e| e.into_inner());
    match scheduler {
        Some(scheduler) => scheduler(id),
        None => crate::reactive::dispatch_main_callback(Box::new(move || poll_task(id))),
    }
}

/// A task started with [`spawn_local`].
///
/// `Copy`; dropping it detaches the task, which runs on. Ask
/// [`abort`](Self::abort) to stop it.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct TaskHandle(u64);

impl TaskHandle {
    /// Stop the task: its future is dropped without being polled again. A no-op
    /// when it has finished or was aborted already. Call on the thread that
    /// spawned it: the handle is `Send`, but tasks are kept per thread, so on
    /// any other thread `abort` finds nothing and does nothing.
    ///
    /// Called from inside the task's own poll, the future is dropped when that
    /// poll returns.
    pub fn abort(&self) {
        abort_task(self.0);
    }

    /// Whether the task has completed, been aborted, panicked, or been dropped
    /// because the component that spawned it unmounted. Call on the thread that
    /// spawned it (on another it answers `true`).
    pub fn is_finished(&self) -> bool {
        !TASKS.with(|t| t.borrow().contains_key(&self.0))
    }
}

/// Run `future` on this (the main/UI) thread, to completion or cancellation.
///
/// The future need not be `Send`: it stays on this thread, so it may capture
/// `Rc`s, [`Signal`](crate::Signal)s and editor handles, and set signals
/// directly. It is not polled here — its first poll is scheduled, and each later
/// poll follows a wake — so `spawn_local` returns before any of its code runs.
///
/// Spawned during a render, the task is dropped when that component unmounts;
/// spawn under [`unowned`](crate::reactive::unowned) for one that must outlive
/// it — see the [module docs](self#lifetime).
pub fn spawn_local(future: impl Future<Output = ()> + 'static) -> TaskHandle {
    refuse_in_worker("spawn_local");
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let owner = current_owner();
    let waker = Arc::new(TaskWaker {
        id,
        scheduled: AtomicBool::new(true),
    });
    let task = Task {
        future: Box::pin(future),
        owner: owner.clone(),
        frame: CallbackFrame::current(),
        waker,
    };
    TASKS.with(|t| t.borrow_mut().insert(id, Slot::Idle(task)));
    if let Some(owner) = owner {
        adopt(owner, id);
    }
    schedule(id);
    TaskHandle(id)
}

/// Record `id` against `owner`, registering the owner's cleanup on its first
/// task. `owner` is the ambient owner, so `on_cleanup_for_ambient_owner` lands on
/// it.
fn adopt(owner: Owner, id: u64) {
    let key = owner.key();
    let first = OWNER_TASKS.with(|o| {
        let mut o = o.borrow_mut();
        let first = !o.contains_key(&key);
        o.entry(key)
            .or_insert_with(|| (owner, HashSet::new()))
            .1
            .insert(id);
        first
    });
    if first {
        crate::reactive::on_cleanup_for_ambient_owner(move || {
            let ids = OWNER_TASKS.with(|o| o.borrow_mut().remove(&key));
            if let Some((_owner, ids)) = ids {
                for id in ids {
                    abort_task(id);
                }
            }
        });
    }
}

fn forget_owner(id: u64, owner: Option<&Owner>) {
    if let Some(owner) = owner {
        OWNER_TASKS.with(|o| {
            if let Some((_, ids)) = o.borrow_mut().get_mut(&owner.key()) {
                ids.remove(&id);
            }
        });
    }
}

fn abort_task(id: u64) {
    let removed = TASKS.with(|t| {
        let mut t = t.borrow_mut();
        match t.get_mut(&id) {
            Some(Slot::Running { aborted, .. }) => {
                *aborted = true;
                None
            }
            Some(Slot::Idle(_)) => t.remove(&id),
            None => None,
        }
    });
    if let Some(Slot::Idle(task)) = removed {
        forget_owner(id, task.owner.as_ref());
        // Dropped outside the table's borrow: the future's captures are user
        // code whose `Drop` may spawn or abort a task.
        drop(task);
    }
}

/// Poll the task `id` once, if it is still alive.
///
/// The backend entry point a [scheduler](set_task_scheduler) calls on the main
/// thread; the default path queues it on the main-thread callback queue. Public
/// so `rinch-web` can call it, not app API — use [`spawn_local`].
#[doc(hidden)]
pub fn poll_task(id: u64) {
    let task = TASKS.with(|t| {
        let mut t = t.borrow_mut();
        match t.get_mut(&id) {
            Some(slot @ Slot::Idle(_)) => match std::mem::replace(
                slot,
                Slot::Running {
                    aborted: false,
                    woken: false,
                },
            ) {
                Slot::Idle(task) => Some(task),
                Slot::Running { .. } => unreachable!(),
            },
            Some(Slot::Running { woken, .. }) => {
                *woken = true;
                None
            }
            None => None,
        }
    });
    let Some(mut task) = task else { return };

    if task.owner.as_ref().is_some_and(|owner| !owner.is_alive()) {
        tracing::debug!("dropping task {id}: the component that spawned it was unmounted");
        finish(id, task);
        return;
    }

    task.waker.scheduled.store(false, Ordering::Release);
    let waker = Waker::from(task.waker.clone());
    // A panic in the poll unwinds past the code below that takes the
    // `Running` marker out; without this the task would stay "being polled"
    // for good. The future itself is dropped by the unwind.
    let unwind = PollUnwind {
        id,
        owner: task.owner.clone(),
    };
    let poll = {
        let Task {
            future,
            owner,
            frame,
            ..
        } = &mut task;
        let _frame = frame.enter();
        let mut cx = Context::from_waker(&waker);
        crate::reactive::batch(|| match owner {
            Some(owner) => owner.run(|| future.as_mut().poll(&mut cx)),
            None => unowned(|| future.as_mut().poll(&mut cx)),
        })
    };

    std::mem::forget(unwind);

    let state = TASKS.with(|t| t.borrow_mut().remove(&id));
    match (poll, state) {
        (
            Poll::Pending,
            Some(Slot::Running {
                aborted: false,
                woken,
            }),
        ) => {
            let waker = task.waker.clone();
            TASKS.with(|t| t.borrow_mut().insert(id, Slot::Idle(task)));
            if woken {
                // The poll a nested drain asked for found this one running.
                waker.scheduled.store(true, Ordering::Release);
                schedule(id);
            }
        }
        _ => finish(id, task),
    }
}

/// Armed around a task's poll: dropped only by an unwind out of it, when it
/// takes the task's `Running` marker and its id out of its owner's set.
struct PollUnwind {
    id: u64,
    owner: Option<Owner>,
}

impl Drop for PollUnwind {
    fn drop(&mut self) {
        let _ = TASKS.try_with(|t| t.try_borrow_mut().map(|mut t| t.remove(&self.id)));
        let _ = OWNER_TASKS.try_with(|o| {
            if let Ok(mut o) = o.try_borrow_mut()
                && let Some(owner) = &self.owner
                && let Some((_, ids)) = o.get_mut(&owner.key())
            {
                ids.remove(&self.id);
            }
        });
    }
}

/// Take a finished or cancelled task out of every table and drop it, outside
/// any borrow.
fn finish(id: u64, task: Task) {
    TASKS.with(|t| {
        t.borrow_mut().remove(&id);
    });
    forget_owner(id, task.owner.as_ref());
    drop(task);
}

/// Install the platform's task scheduler. On wasm32 the web runtime
/// (`rinch::install_web_runtime`, called by every `rinch_web` mount and by
/// `RinchContext::new`) installs one that polls in a microtask; a host with no
/// scheduler gets the main-thread callback queue.
///
/// Tasks already waiting for a poll — spawned before the scheduler existed —
/// are scheduled through it too, so a task spawned before the web runtime is
/// installed is not left on a queue the browser never drains. A backend hook, not app API.
#[doc(hidden)]
pub fn set_task_scheduler(scheduler: TaskScheduler) {
    *SCHEDULER.lock().unwrap_or_else(|e| e.into_inner()) = Some(scheduler);
    let waiting: Vec<u64> = TASKS.with(|t| {
        t.borrow()
            .iter()
            .filter_map(|(id, slot)| match slot {
                Slot::Idle(task) if task.waker.scheduled.load(Ordering::Acquire) => Some(*id),
                _ => None,
            })
            .collect()
    });
    for id in waiting {
        scheduler(id);
    }
}

/// A future that completes `duration` after it is first polled — the async face
/// of [`set_timeout`](crate::set_timeout), with the same scheduling (a shared
/// timer thread natively, `window.setTimeout` on the web).
///
/// Poll it on the main thread. Dropping it before it completes clears its timer.
pub fn sleep(duration: Duration) -> Sleep {
    let ms = duration.as_nanos().div_ceil(1_000_000);
    Sleep {
        delay_ms: u32::try_from(ms).unwrap_or(u32::MAX),
        armed: None,
    }
}

/// The future [`sleep`] returns.
#[must_use = "a sleep does nothing unless awaited"]
pub struct Sleep {
    delay_ms: u32,
    armed: Option<(TimeoutHandle, Rc<SleepState>)>,
}

struct SleepState {
    done: Cell<bool>,
    waker: RefCell<Option<Waker>>,
}

impl Future for Sleep {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let this = self.get_mut();
        match &this.armed {
            Some((_, state)) if state.done.get() => Poll::Ready(()),
            Some((_, state)) => {
                *state.waker.borrow_mut() = Some(cx.waker().clone());
                Poll::Pending
            }
            None => {
                let state = Rc::new(SleepState {
                    done: Cell::new(false),
                    waker: RefCell::new(Some(cx.waker().clone())),
                });
                let fired = state.clone();
                // Unowned: the timer serves this future, whose own lifetime
                // (its task's) decides when it goes; dropping it clears it.
                let handle = unowned(|| {
                    set_timeout(this.delay_ms, move || {
                        fired.done.set(true);
                        let waker = fired.waker.borrow_mut().take();
                        if let Some(waker) = waker {
                            waker.wake();
                        }
                    })
                });
                this.armed = Some((handle, state));
                Poll::Pending
            }
        }
    }
}

impl Drop for Sleep {
    fn drop(&mut self) {
        if let Some((handle, state)) = self.armed.take()
            && !state.done.get()
        {
            clear_timeout(handle);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reactive::Scope;

    fn tasks_of(scope: &Scope) -> Option<usize> {
        let key = scope.owner().key();
        OWNER_TASKS.with(|o| o.borrow().get(&key).map(|(_, ids)| ids.len()))
    }

    /// One cleanup per component, however many tasks it spawns, and a task
    /// that ends leaves its component's set: a component spawning a task per
    /// keystroke must not grow its cleanup list or the table for its whole life.
    #[test]
    fn a_component_registers_one_cleanup_for_all_its_tasks() {
        let scope = Scope::new();
        let before = scope.cleanup_count();
        let handles: Vec<TaskHandle> = (0..50)
            .map(|_| scope.run(|| spawn_local(std::future::pending::<()>())))
            .collect();
        assert_eq!(scope.cleanup_count(), before + 1);
        assert_eq!(tasks_of(&scope), Some(50));

        for handle in &handles {
            handle.abort();
        }
        assert_eq!(tasks_of(&scope), Some(0), "an ended task leaves the set");

        scope.run(|| spawn_local(std::future::pending::<()>()));
        assert_eq!(scope.cleanup_count(), before + 1, "still one cleanup");

        scope.dispose();
        assert_eq!(tasks_of(&scope), None, "the entry goes with the component");
    }

    /// A panic inside a task's poll ends the task: it leaves the table and its
    /// component's set, rather than staying "being polled" for good (where
    /// `is_finished` answered `false` forever and a later abort or wake only
    /// marked the stale entry).
    #[test]
    fn a_task_that_panics_in_its_poll_is_finished_and_forgotten() {
        let scope = Scope::new();
        let handle = scope.run(|| {
            spawn_local(async {
                panic!("a task's own panic");
            })
        });
        assert_eq!(tasks_of(&scope), Some(1));

        let unwound =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| poll_task(handle.0)));
        assert!(
            unwound.is_err(),
            "the panic reaches the drain that polled it"
        );
        assert!(handle.is_finished());
        assert_eq!(tasks_of(&scope), Some(0), "its id left the component's set");
        handle.abort(); // a no-op, not a mark on a stale entry
        assert!(TASKS.with(|t| !t.borrow().contains_key(&handle.0)));
        scope.dispose();
    }
}
