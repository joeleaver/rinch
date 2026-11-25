use std::collections::{BTreeSet, HashMap};
use std::sync::{Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::reactive::run_on_main_thread;

#[derive(Default)]
struct Pending {
    /// Ordered by deadline; ties broken by id.
    by_deadline: BTreeSet<(Instant, u64)>,
    /// Each pending id's deadline, to find it in `by_deadline` on a cancel.
    deadlines: HashMap<u64, Instant>,
}

pub(super) struct Scheduler {
    pending: Mutex<Pending>,
    wake: Condvar,
    /// Hands a due id to the main thread.
    post: Box<dyn Fn(u64) + Send + Sync>,
    /// How many times the thread woke because a deadline came.
    #[cfg(test)]
    pub(super) deadline_wakes: std::sync::atomic::AtomicUsize,
    /// How many ids it handed to the main thread.
    #[cfg(test)]
    pub(super) main_wakes: std::sync::atomic::AtomicUsize,
}

impl Scheduler {
    /// A scheduler with its own thread, living for the rest of the process.
    fn start(post: Box<dyn Fn(u64) + Send + Sync>) -> &'static Scheduler {
        let sched: &'static Scheduler = Box::leak(Box::new(Scheduler {
            pending: Mutex::new(Pending::default()),
            wake: Condvar::new(),
            post,
            #[cfg(test)]
            deadline_wakes: Default::default(),
            #[cfg(test)]
            main_wakes: Default::default(),
        }));
        std::thread::Builder::new()
            .name("rinch-timer".into())
            .spawn(move || sched.run())
            .expect("failed to spawn rinch-timer thread");
        sched
    }

    pub(super) fn schedule(&self, id: u64, delay_ms: u32) {
        self.schedule_at(
            id,
            Instant::now() + Duration::from_millis(u64::from(delay_ms)),
        );
    }

    /// Schedule `a` and `b` on one and the same deadline, `delay` from now.
    #[cfg(test)]
    pub(super) fn schedule_at_for_test(&self, a: u64, b: u64, delay: Duration) {
        let deadline = Instant::now() + delay;
        self.schedule_at(a, deadline);
        self.schedule_at(b, deadline);
    }

    fn schedule_at(&self, id: u64, deadline: Instant) {
        let mut p = self.pending.lock().unwrap();
        p.by_deadline.insert((deadline, id));
        p.deadlines.insert(id, deadline);
        // Wake the thread only if this is now the nearest deadline: it is
        // sleeping until a later one, or indefinitely.
        if p.by_deadline.first() == Some(&(deadline, id)) {
            self.wake.notify_one();
        }
    }

    pub(super) fn cancel(&self, id: u64) {
        let mut p = self.pending.lock().unwrap();
        let Some(deadline) = p.deadlines.remove(&id) else {
            return;
        };
        let was_nearest = p.by_deadline.first() == Some(&(deadline, id));
        p.by_deadline.remove(&(deadline, id));
        // The thread is sleeping until this deadline: have it re-arm for
        // the next one (or sleep indefinitely) instead.
        if was_nearest {
            self.wake.notify_one();
        }
    }

    fn run(&self) {
        let mut p = self.pending.lock().unwrap();
        loop {
            let now = Instant::now();
            // Hand every timer whose deadline has passed to the main thread.
            while let Some(&(deadline, id)) = p.by_deadline.first() {
                if deadline > now {
                    break;
                }
                p.by_deadline.pop_first();
                p.deadlines.remove(&id);
                #[cfg(test)]
                self.main_wakes
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                // Called with the scheduler's mutex held: the cross-thread
                // dispatcher behind `post` must not block (it queues the
                // id and wakes the event loop), or every `set_timeout` and
                // `clear_timeout` on the main thread waits behind it.
                (self.post)(id);
            }
            // Sleep until the next deadline, or indefinitely if none,
            // unless woken because the nearest deadline changed.
            p = match p.by_deadline.first() {
                Some(&(deadline, _)) => {
                    let dur = deadline.saturating_duration_since(Instant::now());
                    let (p, _timeout) = self.wake.wait_timeout(p, dur).unwrap();
                    #[cfg(test)]
                    if _timeout.timed_out() {
                        self.deadline_wakes
                            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    }
                    p
                }
                None => self.wake.wait(p).unwrap(),
            };
        }
    }
}

static SCHEDULER: OnceLock<&'static Scheduler> = OnceLock::new();

fn scheduler() -> &'static Scheduler {
    SCHEDULER.get_or_init(|| {
        // `fire_timeout` resumes the parked (cancel-aware) callback there.
        Scheduler::start(Box::new(|id| {
            run_on_main_thread(move || super::fire_timeout(id))
        }))
    })
}

pub(super) fn schedule(id: u64, delay_ms: u32) {
    scheduler().schedule(id, delay_ms);
}

pub(super) fn cancel(id: u64) {
    // No scheduler yet means no native timer was ever armed.
    if let Some(sched) = SCHEDULER.get() {
        sched.cancel(id);
    }
}

#[cfg(test)]
thread_local! {
    /// A test's own scheduler, used by the timers this thread arms instead
    /// of the process-wide one (other tests in the binary install manual
    /// backends process-wide).
    static TEST_SCHEDULER: std::cell::Cell<Option<&'static Scheduler>> =
        const { std::cell::Cell::new(None) };
}

#[cfg(test)]
pub(super) fn test_scheduler() -> Option<&'static Scheduler> {
    TEST_SCHEDULER.with(|s| s.get())
}

/// Give this thread a scheduler of its own whose due ids arrive on the
/// returned channel, with the instant each was handed over; the test plays
/// the main thread by passing each to `fire_timeout`.
#[cfg(test)]
pub(super) fn install_test_scheduler() -> (
    &'static Scheduler,
    std::sync::mpsc::Receiver<(u64, Instant)>,
) {
    let (tx, rx) = std::sync::mpsc::channel();
    let tx = Mutex::new(tx);
    let sched = Scheduler::start(Box::new(move |id| {
        let _ = tx.lock().unwrap().send((id, Instant::now()));
    }));
    TEST_SCHEDULER.with(|s| s.set(Some(sched)));
    (sched, rx)
}
