//! Cross-thread writes: `send` and `update_send`.

use std::panic::Location;

use super::super::{
    WriteRoute, WriteThread, dispatch_main_callback_for, dispatch_to_main_thread_for, route_write,
    write_thread,
};
use super::Signal;

impl<T: Send + 'static> Signal<T> {
    /// Set the signal from any thread.
    ///
    /// If called from the main thread, behaves identically to [`set()`](Signal::set).
    /// If called from a background thread, automatically dispatches to the main
    /// thread where the signal store lives. A background thread that writes
    /// before the host has started (a worker spawned in `main()` ahead of
    /// `App::run`) has its write queued and applied once the host starts.
    ///
    /// The write goes to the thread that created the signal: where several
    /// threads each host an app (tests running in parallel, each with its own
    /// `rinch::testing::TestApp`), it is applied by that thread's drain, never
    /// another's (issue #2778; see [`mark_host_thread`](crate::mark_host_thread)).
    ///
    /// Requires `T: Send` since the value may cross thread boundaries.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let level = Signal::new(0.0f32);
    ///
    /// std::thread::spawn(move || {
    ///     // This just works — no run_on_main_thread() wrapper needed
    ///     level.send(0.5);
    /// });
    /// ```
    #[track_caller]
    pub fn send(&self, value: T) {
        // Captured here, not inside the closure: the closure runs later on the
        // main thread, where `#[track_caller]` no longer reaches this call site.
        // Without this every dropped cross-thread write would be attributed to
        // one line in this file and the warn-once dedup would silence them all.
        let loc = Location::caller();
        let signal = *self;
        // Applied here when the signal is this thread's, else queued for the
        // thread that created it (issue #2778; see `route_write`).
        match route_write(self.id) {
            WriteRoute::Here => self.set_at(value, "send", loc),
            WriteRoute::Dispatch { owner } => dispatch_to_main_thread_for(
                owner,
                Box::new(move || {
                    signal.set_at(value, "send", loc);
                }),
            ),
            // A worker ahead of the host (issue #1541), or a write to another
            // host thread's signal (#2778): queue it, no dispatcher needed.
            WriteRoute::Queue { owner } => {
                dispatch_main_callback_for(
                    owner,
                    Box::new(move || {
                        // A write for a named thread is drained only there. One
                        // that names none is for the registered main thread: ids
                        // alias across thread-local stores, so a drain on any
                        // other thread (a second test, an unregistered host)
                        // would write a stranger's signal — drop it there, as the
                        // write was before #1541.
                        if owner != 0 || write_thread() == WriteThread::Main {
                            signal.set_at(value, "send", loc);
                        }
                    }),
                )
            }
        }
    }

    /// Update the signal from any thread.
    ///
    /// If called from the main thread, behaves identically to [`update()`](Signal::update).
    /// If called from a background thread, automatically dispatches to the main
    /// thread where the signal store lives.
    ///
    /// Requires the closure to be `Send + 'static` since it may cross thread boundaries.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let items = Signal::new(vec![1, 2, 3]);
    ///
    /// std::thread::spawn(move || {
    ///     items.update_send(|list| list.push(4));
    /// });
    /// ```
    #[track_caller]
    pub fn update_send(&self, f: impl FnOnce(&mut T) + Send + 'static) {
        // See `send` — the location must be captured before the thread hop.
        let loc = Location::caller();
        let signal = *self;
        // See `send` (issues #1541, #2778).
        match route_write(self.id) {
            WriteRoute::Here => self.update_at(f, "update_send", loc),
            WriteRoute::Dispatch { owner } => dispatch_to_main_thread_for(
                owner,
                Box::new(move || {
                    signal.update_at(f, "update_send", loc);
                }),
            ),
            WriteRoute::Queue { owner } => dispatch_main_callback_for(
                owner,
                Box::new(move || {
                    if owner != 0 || write_thread() == WriteThread::Main {
                        signal.update_at(f, "update_send", loc);
                    }
                }),
            ),
        }
    }
}
