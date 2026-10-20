//! The clock that stamps every local transaction (#940).
//!
//! The history plugin merges consecutive typing transactions whose
//! [`Transaction::time`](rinch_editor_core::Transaction::time)s are less than its
//! `new_group_delay` (500 ms) apart, and starts a new undo step otherwise. The
//! core is pure and reads no clock, so the time is the platform's to set, as
//! ProseMirror's view stamps `tr.time = Date.now()`. [`EditorHandle`] stamps every
//! local transaction it dispatches that does not carry a time of its own
//! ([`stamp`]); before #940 nothing did, every time was `0`, and the window was
//! always open — a paragraph typed over a minute was one undo step.
//!
//! The clock is monotonic milliseconds since this thread first asked
//! (`web_time::Instant`: `std::time` on native, `performance.now()` in a
//! browser). An editor and its history live on one thread, so the per-thread
//! origin is never compared across threads.
//!
//! [`EditorHandle`]: crate::EditorHandle

use std::cell::Cell;

use rinch_editor_core::Transaction;
use web_time::Instant;

thread_local! {
    static ORIGIN: Instant = Instant::now();
    static OVERRIDE: Cell<Option<fn() -> u64>> = const { Cell::new(None) };
}

/// Milliseconds on this thread's history clock.
pub(crate) fn now_ms() -> u64 {
    if let Some(clock) = OVERRIDE.with(Cell::get) {
        return clock();
    }
    ORIGIN.with(|origin| origin.elapsed().as_millis() as u64)
}

/// Stamp `tr` with the history clock unless its builder set a time itself
/// ([`Transaction::set_time`]), which is kept.
pub(crate) fn stamp(tr: &mut Transaction) {
    if tr.time() == 0 {
        tr.set_time(now_ms());
    }
}

/// Replace this thread's history clock (`None` restores the real one). A test
/// seam: a fixture that wants to place two keystrokes on either side of the
/// typing-merge window without sleeping.
#[doc(hidden)]
pub fn __set_history_clock(clock: Option<fn() -> u64>) {
    OVERRIDE.with(|c| c.set(clock));
}
