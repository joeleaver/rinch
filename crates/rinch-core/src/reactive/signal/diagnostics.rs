//! The signal's panics, and its warn-once diagnostic for dropped writes.

use std::cell::RefCell;
use std::collections::HashSet;
use std::panic::Location;

use super::super::{SIGNAL_STORE, is_host_tag, signal_owner};

/// Build the off-main-thread panic message used by `set`, `set_if_changed`, and `update`.
///
/// Names the offending method and the exact cross-thread alternative so the panic
/// message itself documents the fix.
#[cold]
#[inline(never)]
pub(super) fn panic_off_main(called: &str, alt: &str) -> ! {
    panic!(
        "Signal::{called}() must run on the main thread. \
         Replace `signal.{called}(...)` with `signal.{alt}(...)` to dispatch \
         across threads, or wrap the call in `rinch::run_on_main_thread(...)` \
         to schedule it manually."
    );
}

/// Build the read-after-free panic message used by `get` and `with`.
///
/// Reads are strict because there is nothing to return: `T` is not `Default`, so
/// a lenient read has no value to hand back. Writes take the other branch — see
/// [`warn_write_dropped`].
#[cold]
#[inline(never)]
pub(super) fn panic_read_freed(called: &str) -> ! {
    panic!(
        "Signal::{called}() on a freed signal. The scope that owned this signal \
         was disposed (its component was removed from the tree) while this handle \
         was still reachable. Guard the read with `signal.is_alive()`, or use \
         `signal.try_{called}(...)` to get an `Option` instead of panicking."
    );
}

/// Warn — at most once per call site — that a write to a freed signal was dropped.
///
/// Writes are lenient because the calling thread frequently cannot know the
/// signal is gone: a detached worker holding a `Copy` handle keeps calling
/// `send`/`update_send` long after the UI that owned the signal was torn down,
/// and panicking there would take down the app for a write nobody was waiting on.
///
/// The caller's location is passed in explicitly rather than read from
/// `Location::caller()` here. `send`/`update_send` dispatch a closure that
/// performs the write *later, on another stack*, where `#[track_caller]`
/// information no longer exists — so they capture their caller's location up
/// front and hand it down. Reading it here would key every cross-thread write
/// to one line inside this file, collapsing the per-call-site dedup into a
/// single global warning.
///
/// A write to a signal **made on another thread** finds nothing here either,
/// and gets a warning of its own (issue #2889): the usual way to reach it is a
/// `send` to a signal of a thread that hosts no app, which `route_write`
/// queues unrouted and a host thread's drain drops. No scope was disposed
/// there, and `is_alive()` on the sender's thread reads the sender's store.
#[cold]
#[inline(never)]
pub(super) fn warn_write_dropped(id: u32, called: &str, loc: &'static Location<'static>) {
    thread_local! {
        /// `(foreign, file, line, column)` of call sites already warned
        /// about. `file()` is `&'static str`, so the key borrows nothing.
        static WARNED: RefCell<HashSet<(bool, &'static str, u32, u32)>> =
            RefCell::new(HashSet::new());
    }

    let foreign = SIGNAL_STORE.with(|store| store.borrow().is_foreign(id));
    let first = WARNED.with(|w| {
        w.borrow_mut()
            .insert((foreign, loc.file(), loc.line(), loc.column()))
    });
    if !first {
        return;
    }

    #[cfg(test)]
    WARN_COUNT.with(|c| c.set(c.get() + 1));

    let (file, line, column) = (loc.file(), loc.line(), loc.column());
    if !foreign {
        tracing::warn!(
            "Signal::{called}() on a freed signal at {file}:{line}:{column} — the write was \
             dropped. The scope that owned this signal was disposed while this handle was \
             still reachable. Check `signal.is_alive()` before writing if the write matters. \
             (Warned once per call site.)",
        );
    } else if is_host_tag(signal_owner(id)) {
        // A direct write from the wrong thread to a host thread's signal:
        // `send` would have been routed there.
        tracing::warn!(
            "Signal::{called}() to a signal made on another thread, at {file}:{line}:{column} \
             — the write was dropped. Only the thread that made a signal holds it. Write it \
             from another thread with `send()` or `update_send()`, which deliver the write to \
             that thread. (Warned once per call site.)",
        );
    } else {
        tracing::warn!(
            "Signal::{called}() to a signal made on another thread, at {file}:{line}:{column} \
             — the write was dropped. That thread hosts no app, so nothing applies writes to \
             its signals. A signal written with `send()` or `update_send()` must be made on \
             the main thread or a host thread (in a test, inside a `TestApp`). (Warned once \
             per call site.)",
        );
    }
}

#[cfg(test)]
thread_local! {
    /// Number of warnings [`warn_write_dropped`] actually emitted, i.e. the
    /// number of *distinct call sites* it saw. Lets the dedup be asserted
    /// without installing a `tracing` subscriber.
    static WARN_COUNT: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// Warnings emitted so far on this thread. Test-only.
#[cfg(test)]
pub(crate) fn warn_count_for_tests() -> u32 {
    WARN_COUNT.with(|c| c.get())
}
