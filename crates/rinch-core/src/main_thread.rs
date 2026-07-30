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

/// A parked continuation, plus the scope that parked it.
struct Parked {
    /// The scope being rendered when this callback was parked, if any.
    ///
    /// `None` means it was parked outside any render — from `main`, a timer, a
    /// detached callback — and has app lifetime. `Some` that has since been
    /// disposed means the component is gone and the callback must not run.
    /// `Owner` is a `Weak`, so this keeps nothing alive.
    owner: Option<crate::reactive::Owner>,
    /// Type-erased `Box<dyn FnOnce(T)>` for the caller's own payload type `T`,
    /// downcast back on resume.
    callback: Box<dyn Any>,
}

thread_local! {
    /// Callbacks parked by [`park_main_callback`], keyed by id, on the thread that
    /// parked them (the main/UI thread). Keeping them thread-local (not global) is
    /// what lets the callback be `!Send`.
    static PARKED: RefCell<HashMap<u64, Parked>> = RefCell::new(HashMap::new());
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
///
/// # Cancelled when the component that parked it unmounts
///
/// A callback parked **during a render** is tied to the scope being built: if
/// that scope is disposed before the callback resumes, [`resume_main_callback`]
/// drops it unrun.
///
/// That is the flip side of the `!Send` capture this whole module exists for —
/// the continuation holds UI state, and disposing a scope frees the signals it
/// holds (issue #141), so resuming afterwards would panic on the first read.
/// Cancelling is also what the caller almost always means: there is no UI left
/// to update.
///
/// The check happens at resume rather than through a cleanup registered on the
/// scope, deliberately: a debounce parks a fresh callback on *every keystroke*,
/// and one cleanup per park would grow without bound for as long as the
/// component lives. Pruning at resume costs nothing and matches how the poll and
/// bounds registries already derive their lifetime from what they drive.
///
/// Park outside any render — or wrap in [`unowned`](crate::reactive::unowned) —
/// for a continuation that must survive its component, such as a save-on-close
/// flush:
///
/// ```ignore
/// rinch_core::reactive::unowned(|| set_timeout(500, move || flush_to_disk()));
/// ```
pub fn park_main_callback<T: 'static>(cb: impl FnOnce(T) + 'static) -> MainCallbackId {
    debug_assert!(
        is_main_thread(),
        "park_main_callback must be called on the main thread; a callback parked \
         off-main can never be resumed (resume runs on the main thread)."
    );
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let boxed: Box<dyn FnOnce(T)> = Box::new(cb);
    let parked = Parked {
        owner: crate::reactive::current_owner(),
        callback: Box::new(boxed),
    };
    PARKED.with(|p| p.borrow_mut().insert(id, parked));
    MainCallbackId(id)
}

/// Resume the callback parked under `id`, passing it `payload`.
///
/// A no-op if the id was already resumed or [`cancelled`](cancel_main_callback),
/// so it is safe to resume at-most-once even if a platform wake races a cancel.
/// The callback is removed from the registry *before* it runs, so it may re-park a
/// fresh callback without re-entrancy trouble.
///
/// Also a no-op if the component that parked the callback has since been
/// unmounted — see [`park_main_callback`]. The callback runs with that component
/// as the ambient owner, so anything it allocates belongs to the component
/// rather than to whatever the event loop happened to be doing.
///
/// Call on the main thread. `T` must match the type used at [`park_main_callback`];
/// a mismatch is a programming error (debug-asserted, ignored in release).
pub fn resume_main_callback<T: 'static>(id: MainCallbackId, payload: T) {
    let entry = PARKED.with(|p| p.borrow_mut().remove(&id.0));
    let Some(entry) = entry else { return };

    if entry.owner.as_ref().is_some_and(|owner| !owner.is_alive()) {
        tracing::debug!(
            "dropping parked callback {}: the component that parked it was unmounted",
            id.0
        );
        // Dropped here, outside the registry borrow above.
        drop(entry);
        return;
    }

    let Ok(cb) = entry.callback.downcast::<Box<dyn FnOnce(T)>>() else {
        debug_assert!(
            false,
            "resume_main_callback: payload type does not match the parked callback"
        );
        return;
    };
    match entry.owner {
        Some(owner) => owner.run(move || (*cb)(payload)),
        None => (*cb)(payload),
    }
}

/// Drop the callback parked under `id` without running it.
///
/// Idempotent, and harmless after the callback has already run (or was never
/// parked).
pub fn cancel_main_callback(id: MainCallbackId) {
    // Taken out first: the callback closes over arbitrary user state whose
    // `Drop` may itself park or cancel a callback, and dropping it inside the
    // `borrow_mut` would make that a `BorrowMutError`.
    let entry = PARKED.with(|p| p.borrow_mut().remove(&id.0));
    drop(entry);
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

    /// A continuation parked during a render is dropped unrun once the
    /// component that parked it is unmounted (issue #141).
    ///
    /// Without this, the idiomatic `set_timeout(500, move || label.set(fmt(count.get())))`
    /// panics on the read when the component unmounts before the deadline —
    /// disposal has freed `count` by then.
    #[test]
    fn a_callback_parked_during_a_render_is_dropped_when_its_scope_is_disposed() {
        use crate::reactive::{Scope, Signal};

        let ran = Rc::new(Cell::new(false));
        let scope = Scope::new();

        let r = ran.clone();
        let id = scope.run(|| {
            let owned = Signal::new(7);
            park_main_callback::<()>(move |()| {
                // Would panic on a freed signal if this were allowed to run.
                let _ = owned.get();
                r.set(true);
            })
        });

        scope.dispose();
        resume_main_callback(id, ());

        assert!(
            !ran.get(),
            "a parked continuation must not outlive the component that parked it"
        );
    }

    /// The escape hatch, and the non-breaking half: parked outside any render —
    /// or under `unowned` — the callback keeps app lifetime.
    #[test]
    fn a_callback_parked_outside_a_render_still_resumes() {
        use crate::reactive::{Scope, unowned};

        // No ambient owner at all.
        let ran = Rc::new(Cell::new(false));
        let r = ran.clone();
        let id = park_main_callback::<()>(move |()| r.set(true));
        resume_main_callback(id, ());
        assert!(ran.get(), "an ownerless callback resumes normally");

        // Explicitly opted out from inside a render.
        let escaped = Rc::new(Cell::new(false));
        let scope = Scope::new();
        let e = escaped.clone();
        let id = scope.run(|| unowned(|| park_main_callback::<()>(move |()| e.set(true))));

        scope.dispose();
        resume_main_callback(id, ());

        assert!(
            escaped.get(),
            "`unowned` must detach the callback from the render that parked it"
        );
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
