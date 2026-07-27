//! Polled signals: bridge high-frequency external sources into the reactive graph
//! without spawning a dedicated thread per source.
//!
//! See [`poll_signal`] for the typical entry point. The runtime drains the registry
//! once per frame via [`drain_polls`].

use std::cell::RefCell;
use web_time::Instant;

use super::Signal;
use super::is_main_thread;

/// How often a polled signal should sample its source.
#[derive(Clone, Copy, Debug)]
pub enum PollRate {
    /// Sample on every render frame. Suitable for sources cheap enough that
    /// you don't care about extra reads (e.g., a single atomic load).
    EveryFrame,
    /// Sample at approximately this many times per second. Internally rounded
    /// down to a millisecond interval (e.g., `Hz(60)` becomes ~16ms).
    Hz(u32),
    /// Sample no more often than this interval in milliseconds.
    Millis(u64),
}

impl PollRate {
    fn interval_ms(self) -> u64 {
        match self {
            PollRate::EveryFrame => 0,
            PollRate::Hz(hz) => 1000 / hz.max(1) as u64,
            PollRate::Millis(ms) => ms,
        }
    }
}

struct PollEntry {
    interval_ms: u64,
    last_fired: Instant,
    /// Fires the poll. Returns `false` once the driven signal is gone, which is
    /// [`drain_polls`]'s signal to drop this entry (issue #141, SD3).
    run: Box<dyn FnMut() -> bool>,
    /// Whether the driven signal is still live, *without* firing the poll.
    ///
    /// Separate from `run` so an entry that is not due this frame can still be
    /// reaped. Judging liveness only on a fire would tie reclamation latency to
    /// the poll's own interval — a `PollRate::Millis(60_000)` entry would sit on
    /// its dead signal, and its captured closure, for a minute.
    alive: Box<dyn Fn() -> bool>,
}

thread_local! {
    static POLL_REGISTRY: RefCell<Vec<PollEntry>> = const { RefCell::new(Vec::new()) };
}

/// Bridge a high-frequency external source (audio-thread atomic, network status flag,
/// hardware sensor, …) into a `Signal<T>` without a dedicated polling thread.
///
/// The source closure is invoked on the **main thread** at most every
/// `rate.interval_ms()` milliseconds, immediately before each frame is painted.
/// The signal is only notified when the value actually changes
/// (uses [`Signal::set_if_changed`]), so subscribers don't re-run on idle reads.
///
/// `poll_signal` itself must be called from the main thread; the source closure
/// also runs on the main thread, so it does not need `Send`. It typically captures
/// an `Arc<AtomicU64>` (or similar `Send + Sync` shared state) updated from a
/// background thread.
///
/// # Example
///
/// ```ignore
/// use std::sync::Arc;
/// use std::sync::atomic::{AtomicU64, Ordering};
/// use rinch_core::reactive::{poll_signal, PollRate};
///
/// // Sample clock written from the audio thread.
/// let sample_clock: Arc<AtomicU64> = Arc::new(AtomicU64::new(0));
///
/// // 60Hz mirror into the UI without a polling thread.
/// let sc = Arc::clone(&sample_clock);
/// let playhead = poll_signal(move || sc.load(Ordering::Relaxed), PollRate::Hz(60));
///
/// // Use `playhead` like any other Signal in rsx.
/// ```
///
/// # Lifetime
///
/// **A poll lives exactly as long as the signal it drives.** [`drain_polls`]
/// drops the entry on the first drain after the signal is gone — whether or not
/// the poll was due to fire — so the registry self-prunes and the source
/// closure, along with everything it captures, is released with it.
///
/// Registering at startup therefore still gives application lifetime, because a
/// signal created outside any render has no owning scope and is never freed.
/// Registering inside a component ties the poll to that component's scope: when
/// it is removed from the tree, its signal is freed and the poll stops on the
/// next drain. Neither case needs an external flag.
pub fn poll_signal<T, F>(mut source: F, rate: PollRate) -> Signal<T>
where
    T: PartialEq + Clone + 'static,
    F: FnMut() -> T + 'static,
{
    assert!(
        is_main_thread(),
        "poll_signal must be called from the main thread; the source closure runs \
         on the main thread and is not Send"
    );

    let initial = source();
    let signal = Signal::new(initial);

    let run = move || {
        // Check first so a dead signal never runs the source — the source may
        // have observable side effects, and a lenient `set_if_changed` on a
        // freed signal would otherwise warn on every frame forever.
        if !signal.is_alive() {
            return false;
        }
        let value = source();
        signal.set_if_changed(value);
        // Re-check: the source (or an effect it woke) may have disposed the
        // scope that owned the signal.
        signal.is_alive()
    };

    POLL_REGISTRY.with(|reg| {
        reg.borrow_mut().push(PollEntry {
            interval_ms: rate.interval_ms(),
            // Start `last_fired` in the past so the first frame fires immediately
            // if the source has already changed before paint runs.
            last_fired: Instant::now()
                .checked_sub(std::time::Duration::from_secs(1))
                .unwrap_or_else(Instant::now),
            run: Box::new(run),
            alive: Box::new(move || signal.is_alive()),
        });
    });

    signal
}

/// Fire any polls whose interval has elapsed, dropping those whose signal has
/// been freed.
///
/// Called by the rinch runtime once per painted frame, before
/// [`drain_main_queue`](super) and layout resolution. Application code should not
/// need to invoke this directly.
///
/// The registry is moved out for the duration of the drain, so source closures
/// run with **no borrow held**. That makes re-entrancy safe: a source that calls
/// [`poll_signal`], or that wakes an effect which does, registers into the empty
/// registry and is spliced back in afterwards instead of panicking with a
/// `BorrowMutError`. A re-entrant `drain_polls` sees an empty registry and is a
/// no-op, so no poll can fire twice in a frame.
///
/// The splice-back runs even if a source panics, so one bad poll cannot take the
/// whole registry with it as the unwind passes through.
pub fn drain_polls() {
    if !is_main_thread() {
        return;
    }
    let now = Instant::now();

    /// Returns the drained entries to `POLL_REGISTRY` on the way out — including
    /// while unwinding from a panicking source. Without this the moved-out
    /// `Vec` would simply be dropped and every poll on the thread would vanish.
    struct SpliceBack(Vec<PollEntry>);

    impl Drop for SpliceBack {
        fn drop(&mut self) {
            let mut entries = std::mem::take(&mut self.0);
            // `try_with`/`try_borrow_mut`: this runs on the unwind path too, so
            // it must not panic-in-panic if TLS is gone or already borrowed.
            let _ = POLL_REGISTRY.try_with(|reg| {
                if let Ok(mut reg) = reg.try_borrow_mut() {
                    // Anything registered re-entrantly during the drain landed
                    // in `reg`; keep it, ordered after the survivors.
                    entries.append(&mut reg);
                    *reg = entries;
                }
            });
        }
    }

    let mut drained = SpliceBack(POLL_REGISTRY.with(|reg| std::mem::take(&mut *reg.borrow_mut())));

    drained.0.retain_mut(|entry| {
        let elapsed_ms = now.duration_since(entry.last_fired).as_millis() as u64;
        if entry.interval_ms == 0 || elapsed_ms >= entry.interval_ms {
            entry.last_fired = now;
            (entry.run)()
        } else {
            // Not due this frame, but still reap it if its signal has died —
            // otherwise reclamation latency would be the poll's own interval.
            (entry.alive)()
        }
    });
}

/// Number of registered polls on this thread. Test-only: the self-pruning
/// contract is about entries *disappearing*, which is otherwise unobservable.
#[cfg(test)]
pub(crate) fn registry_len_for_tests() -> usize {
    POLL_REGISTRY.with(|reg| reg.borrow().len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn poll_signal_initial_value() {
        let counter = std::cell::Cell::new(0i32);
        let signal = poll_signal(move || counter.get(), PollRate::EveryFrame);
        assert_eq!(signal.get(), 0);
    }

    #[test]
    fn drain_polls_propagates_changes() {
        let counter = std::rc::Rc::new(std::cell::Cell::new(0i32));
        let c = std::rc::Rc::clone(&counter);
        let signal = poll_signal(move || c.get(), PollRate::EveryFrame);
        assert_eq!(signal.get(), 0);

        counter.set(42);
        drain_polls();
        assert_eq!(signal.get(), 42);

        // No change → set_if_changed no-ops, value stays the same.
        drain_polls();
        assert_eq!(signal.get(), 42);

        counter.set(7);
        drain_polls();
        assert_eq!(signal.get(), 7);
    }

    #[test]
    fn rate_interval_ms() {
        assert_eq!(PollRate::EveryFrame.interval_ms(), 0);
        assert_eq!(PollRate::Hz(60).interval_ms(), 16);
        assert_eq!(PollRate::Hz(1000).interval_ms(), 1);
        assert_eq!(PollRate::Millis(33).interval_ms(), 33);
    }
}

#[cfg(test)]
mod lifetime_tests {
    use super::*;
    use std::cell::Cell;
    use std::rc::Rc;

    #[test]
    fn a_poll_is_dropped_once_its_signal_is_freed() {
        let reads = Rc::new(Cell::new(0));
        let r = Rc::clone(&reads);
        let signal = poll_signal(
            move || {
                r.set(r.get() + 1);
                r.get()
            },
            PollRate::EveryFrame,
        );

        assert_eq!(registry_len_for_tests(), 1);
        drain_polls();
        let reads_while_alive = reads.get();
        assert!(
            reads_while_alive >= 2,
            "the source ran while the signal lived"
        );

        signal.free_for_tests();

        drain_polls();
        assert_eq!(
            registry_len_for_tests(),
            0,
            "a poll whose signal is gone must be removed, not left spinning"
        );
        assert_eq!(
            reads.get(),
            reads_while_alive,
            "the source must not run for a freed signal"
        );

        // Still gone, and still no work, on later frames.
        drain_polls();
        assert_eq!(registry_len_for_tests(), 0);
        assert_eq!(reads.get(), reads_while_alive);
    }

    #[test]
    fn a_live_poll_registered_beside_a_dead_one_survives() {
        let doomed = poll_signal(|| 1, PollRate::EveryFrame);
        let survivor_ticks = Rc::new(Cell::new(0));
        let t = Rc::clone(&survivor_ticks);
        let _survivor = poll_signal(
            move || {
                t.set(t.get() + 1);
                t.get()
            },
            PollRate::EveryFrame,
        );
        assert_eq!(registry_len_for_tests(), 2);

        doomed.free_for_tests();
        drain_polls();

        assert_eq!(registry_len_for_tests(), 1, "only the dead entry is reaped");
        let after = survivor_ticks.get();
        drain_polls();
        assert!(survivor_ticks.get() > after, "the survivor keeps firing");
    }

    #[test]
    fn a_poll_source_may_register_another_poll() {
        // Pre-fix this was a BorrowMutError: `drain_polls` held
        // `POLL_REGISTRY.borrow_mut()` across the source closure, and
        // `poll_signal` takes `borrow_mut()` to push.
        //
        // The nested registration must happen on call #2. Call #1 is
        // `poll_signal`'s own initial read, which runs *before* it touches the
        // registry — registering from there would exercise nothing.
        let calls = Rc::new(Cell::new(0));
        let c = Rc::clone(&calls);
        let _outer = poll_signal(
            move || {
                c.set(c.get() + 1);
                if c.get() == 2 {
                    let _inner = poll_signal(|| 0u8, PollRate::EveryFrame);
                }
                0u8
            },
            PollRate::EveryFrame,
        );
        assert_eq!(calls.get(), 1, "registration read the source once");
        assert_eq!(registry_len_for_tests(), 1);

        drain_polls();

        assert_eq!(calls.get(), 2, "the drain fired the outer poll");
        assert_eq!(
            registry_len_for_tests(),
            2,
            "the poll registered during the drain is spliced back in"
        );

        // And the newcomer participates from the next frame on.
        drain_polls();
        assert_eq!(registry_len_for_tests(), 2);
    }

    #[test]
    fn a_reentrant_drain_is_a_no_op_rather_than_a_double_fire() {
        let ticks = Rc::new(Cell::new(0));
        let t = Rc::clone(&ticks);
        let _p = poll_signal(
            move || {
                t.set(t.get() + 1);
                // Re-entering the drain must not re-fire this very poll.
                drain_polls();
                t.get()
            },
            PollRate::EveryFrame,
        );
        // Registration itself invokes the source once for the initial value.
        let after_register = ticks.get();

        drain_polls();

        assert_eq!(
            ticks.get(),
            after_register + 1,
            "the poll fired exactly once for this frame"
        );
        assert_eq!(registry_len_for_tests(), 1);
    }
}

#[cfg(test)]
mod resilience_tests {
    use super::*;
    use std::cell::Cell;
    use std::rc::Rc;

    #[test]
    fn a_panicking_source_does_not_destroy_the_registry() {
        // `drain_polls` moves the registry out with `mem::take`. Without a
        // splice-back on the unwind path, a single panicking source would drop
        // the moved-out Vec and silently unregister every poll on the thread.
        let survivor_ticks = Rc::new(Cell::new(0));
        let t = Rc::clone(&survivor_ticks);
        let _survivor = poll_signal(
            move || {
                t.set(t.get() + 1);
                t.get()
            },
            PollRate::EveryFrame,
        );

        // Panic on call #2. Call #1 is `poll_signal`'s own initial read, which
        // happens before the registry is ever moved out — panicking there would
        // exercise nothing.
        let calls = Rc::new(Cell::new(0));
        let c = Rc::clone(&calls);
        let _bad = poll_signal(
            move || {
                c.set(c.get() + 1);
                // Exactly one panic: the follow-up drain below must be able to
                // fire this entry again to prove the registry still works.
                if c.get() == 2 {
                    panic!("poll source blew up");
                }
                0u8
            },
            PollRate::EveryFrame,
        );
        assert_eq!(calls.get(), 1);
        assert_eq!(registry_len_for_tests(), 2);

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(drain_polls));
        assert!(result.is_err(), "the panic propagates to the caller");

        assert_eq!(
            registry_len_for_tests(),
            2,
            "both polls are still registered after the unwind"
        );

        // And the registry is still functional.
        let before = survivor_ticks.get();
        drain_polls();
        assert!(survivor_ticks.get() > before, "the survivor still fires");
    }

    #[test]
    fn a_not_due_poll_is_still_reaped_once_its_signal_dies() {
        // Liveness must not be judged only on a fire, or a slow poll would hold
        // its dead signal's closure for a whole interval.
        let signal = poll_signal(|| 0u8, PollRate::Millis(60_000));
        assert_eq!(registry_len_for_tests(), 1);

        // Registration backdates `last_fired` by 1s, so at a 60s interval this
        // entry is definitively not due.
        signal.free_for_tests();
        drain_polls();

        assert_eq!(
            registry_len_for_tests(),
            0,
            "reclamation must not wait for the poll's own interval"
        );
    }
}
