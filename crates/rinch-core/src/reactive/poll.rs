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
    run: Box<dyn FnMut()>,
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
/// Polls registered today live for the lifetime of the application; there is no
/// unregister yet. This matches the typical "register once at startup" pattern;
/// if you need scoped polling, gate the source closure on an external flag.
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
        let value = source();
        signal.set_if_changed(value);
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
        });
    });

    signal
}

/// Fire any polls whose interval has elapsed.
///
/// Called by the rinch runtime once per painted frame, before
/// [`drain_main_queue`](super) and layout resolution. Application code should not
/// need to invoke this directly.
pub fn drain_polls() {
    if !is_main_thread() {
        return;
    }
    let now = Instant::now();
    POLL_REGISTRY.with(|reg| {
        let mut polls = reg.borrow_mut();
        for entry in polls.iter_mut() {
            let elapsed_ms = now.duration_since(entry.last_fired).as_millis() as u64;
            if entry.interval_ms == 0 || elapsed_ms >= entry.interval_ms {
                entry.last_fired = now;
                (entry.run)();
            }
        }
    });
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
