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
//! The clock is monotonic milliseconds since this thread first asked, read
//! from rinch's clock, [`rinch_core::timer::now`] (#2842): `std::time` on
//! native, `performance.now()` in a browser, and the virtual clock under the
//! test harness (`rinch::testing::TestApp`), so a test's `advance_time` opens
//! and closes the merge window exactly. An editor and its history live on one
//! thread, so the per-thread origin is never compared across threads.
//!
//! [`EditorHandle`]: crate::EditorHandle

use std::cell::Cell;

use rinch_core::timer::Instant;
use rinch_editor_core::Transaction;

thread_local! {
    static ORIGIN: Instant = rinch_core::timer::now();
    static OVERRIDE: Cell<Option<fn() -> u64>> = const { Cell::new(None) };
}

/// Milliseconds on this thread's history clock: what [`EditorHandle`] stamps a
/// transaction with. An app that sets a transaction's time itself
/// ([`Transaction::set_time`]) must read it from here — the history compares
/// times as numbers, so a time on another clock (epoch milliseconds, say) keeps
/// the next keystroke from merging with that transaction.
///
/// [`EditorHandle`]: crate::EditorHandle
pub fn history_now_ms() -> u64 {
    now_ms()
}

pub(crate) fn now_ms() -> u64 {
    if let Some(clock) = OVERRIDE.with(Cell::get) {
        return clock();
    }
    // Saturating: a virtual clock installed after the origin was taken may
    // read an instant before it.
    ORIGIN.with(|origin| {
        rinch_core::timer::now()
            .saturating_duration_since(*origin)
            .as_millis() as u64
    })
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
