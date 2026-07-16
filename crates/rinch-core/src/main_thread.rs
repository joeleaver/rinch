//! Main-thread callback parking + resumption.
//!
//! This is the one concurrency primitive every off-the-main-thread rinch API
//! needs: **park a `!Send` continuation on the main (UI) thread, do work
//! off-thread, then resume that continuation *by id* on the main thread with a
//! `Send` payload.** Only the id ([`MainCallbackId`], a `u64`) and the payload
//! cross the thread boundary; the callback itself — which typically captures
//! `!Send` UI state (`Rc`, [`Signal`](crate::Signal), an editor handle) — never
//! leaves the thread it was parked on and so needn't be `Send`.
//!
//! It is the shared substrate under [`set_timeout`](crate::set_timeout) (payload
//! `()`) and `rinch-http`'s asynchronous `fetch` (payload `Result<Response, _>`):
//! a background thread computes the `Send` result, hops back onto the main thread
//! via [`run_on_main_thread`](crate::run_on_main_thread), and calls
//! [`resume_main_callback`].
//!
//! ```ignore
//! // Park the (!Send) continuation on the main thread; keep the id.
//! let id = park_main_callback::<String>(move |body| label.set(body));
//! std::thread::spawn(move || {
//!     let body = fetch_blocking();               // off-thread work -> Send payload
//!     run_on_main_thread(move || resume_main_callback(id, body));
//! });
//! ```

use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::reactive::is_main_thread;

thread_local! {
    /// Callbacks parked by [`park_main_callback`], keyed by id, on the thread that
    /// parked them (the main/UI thread). Type-erased: each entry holds a
    /// `Box<dyn FnOnce(T)>` for the caller's own payload type `T`, downcast back on
    /// resume. Keeping them thread-local (not global) is what lets the callback be
    /// `!Send`.
    static PARKED: RefCell<HashMap<u64, Box<dyn Any>>> = RefCell::new(HashMap::new());
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// An id for a callback parked with [`park_main_callback`].
///
/// `Copy`/`Send`, so it can travel to a worker thread that later resumes the
/// callback on the main thread. Pass it to [`resume_main_callback`] to run the
/// callback, or [`cancel_main_callback`] to drop it unrun.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct MainCallbackId(u64);

impl MainCallbackId {
    /// The raw id, for backends that must round-trip it through a plain integer —
    /// e.g. a `fn(u64, u32)` timer scheduler, or a JS `setTimeout` shim that can
    /// only carry a number.
    pub fn raw(self) -> u64 {
        self.0
    }

    /// Reconstruct an id from its raw value (see [`raw`](Self::raw)).
    pub fn from_raw(id: u64) -> Self {
        MainCallbackId(id)
    }
}

/// Park a callback on the current (main/UI) thread, returning its id.
///
/// The callback is stored on this thread and invoked here by
/// [`resume_main_callback`], so it need not be `Send` and may freely capture
/// `!Send` UI state. `T` is the payload type a later `resume_main_callback::<T>`
/// will deliver.
///
/// Must be called on the main thread: [`resume_main_callback`] looks the callback
/// up in *its* thread's registry, so a callback parked off-main could never be
/// resumed (debug-asserted).
pub fn park_main_callback<T: 'static>(cb: impl FnOnce(T) + 'static) -> MainCallbackId {
    debug_assert!(
        is_main_thread(),
        "park_main_callback must be called on the main thread; a callback parked \
         off-main can never be resumed (resume runs on the main thread)."
    );
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let boxed: Box<dyn FnOnce(T)> = Box::new(cb);
    PARKED.with(|p| p.borrow_mut().insert(id, Box::new(boxed)));
    MainCallbackId(id)
}

/// Resume the callback parked under `id`, passing it `payload`.
///
/// A no-op if the id was already resumed or [`cancelled`](cancel_main_callback),
/// so it is safe to resume at-most-once even if a platform wake races a cancel.
/// The callback is removed from the registry *before* it runs, so it may re-park a
/// fresh callback without re-entrancy trouble.
///
/// Call on the main thread. `T` must match the type used at [`park_main_callback`];
/// a mismatch is a programming error (debug-asserted, ignored in release).
pub fn resume_main_callback<T: 'static>(id: MainCallbackId, payload: T) {
    let entry = PARKED.with(|p| p.borrow_mut().remove(&id.0));
    let Some(entry) = entry else { return };
    match entry.downcast::<Box<dyn FnOnce(T)>>() {
        Ok(cb) => (*cb)(payload),
        Err(_) => debug_assert!(
            false,
            "resume_main_callback: payload type does not match the parked callback"
        ),
    }
}

/// Drop the callback parked under `id` without running it.
///
/// Idempotent, and harmless after the callback has already run (or was never
/// parked).
pub fn cancel_main_callback(id: MainCallbackId) {
    PARKED.with(|p| {
        p.borrow_mut().remove(&id.0);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::rc::Rc;

    #[test]
    fn park_resume_delivers_payload_once() {
        let seen = Rc::new(Cell::new(0u32));
        // `Rc` capture — the whole point of parking main-thread-side; this would
        // not compile if the callback were required to be `Send`.
        let s = seen.clone();
        let id = park_main_callback::<u32>(move |n| s.set(s.get() + n));

        resume_main_callback(id, 5u32);
        assert_eq!(seen.get(), 5, "callback runs with its payload");

        // One-shot: a second resume finds nothing parked and no-ops.
        resume_main_callback(id, 100u32);
        assert_eq!(seen.get(), 5, "callback is at-most-once");
    }

    #[test]
    fn cancel_prevents_the_callback() {
        let ran = Rc::new(Cell::new(false));
        let r = ran.clone();
        let id = park_main_callback::<()>(move |()| r.set(true));

        cancel_main_callback(id);
        resume_main_callback(id, ());
        assert!(!ran.get(), "cancelled callback must not run");

        // Cancelling again (or after a run) is a harmless no-op.
        cancel_main_callback(id);
    }

    #[test]
    fn ids_are_distinct_and_independent() {
        let log = Rc::new(RefCell::new(Vec::<&'static str>::new()));
        let l1 = log.clone();
        let a = park_main_callback::<()>(move |()| l1.borrow_mut().push("a"));
        let l2 = log.clone();
        let b = park_main_callback::<()>(move |()| l2.borrow_mut().push("b"));
        assert_ne!(a, b);

        // Resume out of order; each id maps to exactly its own callback.
        resume_main_callback(b, ());
        resume_main_callback(a, ());
        assert_eq!(*log.borrow(), vec!["b", "a"]);
    }
}
