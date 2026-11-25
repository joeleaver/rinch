use std::cell::RefCell;
use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::time::Duration;

use crate::main_thread::{MainCallbackId, cancel_main_callback};

struct Clock {
    /// The wall-clock instant the clock started at: the epoch of the
    /// instants [`now`] answers.
    start: web_time::Instant,
    /// The virtual time since `start`.
    elapsed: Duration,
    /// Waiting timers by deadline, then by the order they were armed.
    heap: BinaryHeap<Reverse<(Duration, u64, u64)>>,
    /// The next arming's sequence number.
    seq: u64,
    /// How many installs are outstanding.
    depth: usize,
}

thread_local! {
    static CLOCK: RefCell<Option<Clock>> = const { RefCell::new(None) };
}

/// Install the thread's virtual clock at zero, or nest in the one there.
pub fn install() {
    CLOCK.with(|c| {
        let mut c = c.borrow_mut();
        match c.as_mut() {
            Some(clock) => clock.depth += 1,
            None => {
                *c = Some(Clock {
                    start: web_time::Instant::now(),
                    elapsed: Duration::ZERO,
                    heap: BinaryHeap::new(),
                    seq: 0,
                    depth: 1,
                })
            }
        }
    });
}

/// Undo one [`install`]. The last one removes the clock and cancels the
/// timers still waiting on it.
pub fn uninstall() {
    let gone = CLOCK.with(|c| {
        let mut c = c.borrow_mut();
        let clock = c.as_mut()?;
        clock.depth -= 1;
        if clock.depth > 0 {
            return None;
        }
        c.take()
    });
    // Cancelled outside the borrow: dropping a callback can run code.
    if let Some(clock) = gone {
        for Reverse((_, _, id)) in clock.heap {
            cancel_main_callback(MainCallbackId::from_raw(id));
        }
    }
}

/// Whether this thread has a virtual clock.
pub fn is_installed() -> bool {
    CLOCK.with(|c| c.borrow().is_some())
}

/// The virtual time since the clock was installed; `None` without one.
pub fn elapsed() -> Option<Duration> {
    CLOCK.with(|c| c.borrow().as_ref().map(|c| c.elapsed))
}

/// The clock's instant now; `None` without one.
pub(super) fn now() -> Option<web_time::Instant> {
    CLOCK.with(|c| c.borrow().as_ref().map(|c| c.start + c.elapsed))
}

/// Queue timer `id` on the virtual clock, if this thread has one.
pub(super) fn schedule(id: u64, delay_ms: u32) -> bool {
    CLOCK.with(|c| {
        let mut c = c.borrow_mut();
        let Some(clock) = c.as_mut() else {
            return false;
        };
        let deadline = clock.elapsed + Duration::from_millis(u64::from(delay_ms));
        let seq = clock.seq;
        clock.seq += 1;
        clock.heap.push(Reverse((deadline, seq, id)));
        true
    })
}

/// Take timer `id` off the virtual clock, if this thread has one (then
/// `true`, whether or not `id` was waiting on it).
pub(super) fn cancel(id: u64) -> bool {
    CLOCK.with(|c| {
        let mut c = c.borrow_mut();
        let Some(clock) = c.as_mut() else {
            return false;
        };
        clock.heap.retain(|&Reverse((_, _, waiting))| waiting != id);
        true
    })
}

/// The sequence number the next timer armed will get: a timer armed from
/// now on has one at least this.
pub fn next_seq() -> u64 {
    CLOCK.with(|c| c.borrow().as_ref().map_or(0, |c| c.seq))
}

/// Take the earliest waiting timer due at or before `until` (virtual
/// time) and armed before sequence number `armed_before`, moving the clock
/// to its deadline, and answer its id for [`super::fire_timeout`]. Timers
/// due together come in the order they were armed. `None` when there is
/// none (or no clock).
pub fn take_due(until: Duration, armed_before: u64) -> Option<u64> {
    CLOCK.with(|c| {
        let mut c = c.borrow_mut();
        let clock = c.as_mut()?;
        let &Reverse((deadline, seq, id)) = clock.heap.peek()?;
        if deadline > until || seq >= armed_before {
            return None;
        }
        clock.heap.pop();
        clock.elapsed = clock.elapsed.max(deadline);
        Some(id)
    })
}

/// Move the clock forward to `to` (it never goes back).
pub fn advance_to(to: Duration) {
    CLOCK.with(|c| {
        if let Some(clock) = c.borrow_mut().as_mut() {
            clock.elapsed = clock.elapsed.max(to);
        }
    });
}
