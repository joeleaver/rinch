//! Who a signal read subscribes: the **current observer** (issue #3047).
//!
//! A read inside an effect's body or a memo's computation subscribes that
//! effect or memo, and nothing else. The runtime keeps one slot,
//! [`Runtime::observer`](super::Runtime), not a stack: each run installs its
//! own id and puts back whatever was there when it returns.
//!
//! It used to be a stack, and an effect run *pushed* its id over the runs in
//! progress beneath it. Runs nest routinely: outside a batch — which includes
//! every effect a batch's exit flush runs — a write flushes the effects it
//! wakes right there, inside the writer's run, and a memo computes inside the
//! read that wants it. [`untracked`] popped one frame, so an `untracked` read
//! inside a nested run subscribed the run *below* it. Every `if let` and
//! `match` swap renders its branch under `untracked`, re-reading the scrutinee
//! to bind its names, so an effect that wrote the signal a branch showed was
//! subscribed to that signal and re-ran when it changed (re-ofscp's admin
//! console sent each request twice). The same leak had been patched site by
//! site before — #491 balanced one `untracked` per render boundary, #932 made
//! the app-callback sites suspend the whole stack — and stayed open wherever a
//! site had not been patched. With one slot it cannot happen anywhere: there
//! is no frame below to expose.
//!
//! The rule, stated once: **an observer depends on exactly what was read while
//! it was current** — during its own run, outside every `untracked` region,
//! and not during any other observer's run nested inside it.

use super::{ObserverId, RUNTIME, restore};

/// The observer a read made now subscribes, if any: the current run's, unless
/// a suspension taken during that run is still alive.
#[inline]
pub(super) fn current_observer() -> Option<ObserverId> {
    RUNTIME.with(|rt| {
        let rt = rt.borrow();
        if rt.suspensions.count == 0 {
            rt.observer
        } else {
            None
        }
    })
}

/// The suspensions in force, counted per **frame** (one observer run, or the
/// top level outside every run).
///
/// A count rather than a saved observer, so suspensions end in any order: the
/// observer is hidden while at least one suspension taken in its run is alive,
/// whichever is released first. Each suspension remembers the frame it was
/// taken in, and its release counts only there:
///
/// - released in its own frame, it lowers the count;
/// - released after its frame ended (the guard outlived the run, held in a
///   struct or a thread-local), it does nothing — the count ended with the
///   frame, so a finished run's observer can never come back as current;
/// - released while a run nested inside its frame is current (the guard moved
///   into that run), it is recorded in [`deferred`](Self::deferred) and taken
///   off the count when its frame is current again.
///
/// Frames nest strictly (each run's [`ObserverGuard`] is a scoped value), and
/// frame ids grow, so the frames still alive are the current one and frames
/// with *smaller* ids; a deferred release for a larger id than the current
/// frame's belongs to a frame that has ended.
#[derive(Debug, Default)]
pub(crate) struct Suspensions {
    /// Suspensions alive in the current frame.
    count: u32,
    /// The current frame's id; `0` is the top level.
    frame: u64,
    /// The last frame id handed out.
    last_frame: u64,
    /// Frames whose suspensions were released while a run nested inside them
    /// was current, one entry per release.
    deferred: Vec<u64>,
}

impl Suspensions {
    /// Take off the count the releases recorded for the frame now current, and
    /// forget those recorded for frames that have ended.
    fn settle_deferred(&mut self) {
        let current = self.frame;
        let mut released = 0u32;
        self.deferred.retain(|&frame| {
            if frame == current {
                released += 1;
            }
            frame < current
        });
        debug_assert!(
            released <= self.count,
            "a frame released more suspensions than it took"
        );
        self.count = self.count.saturating_sub(released);
    }
}

/// Makes an observer current for one run, and puts back the one it displaced
/// on drop — including while unwinding.
///
/// A bare set/restore pair leaks the id when user code panics between them,
/// and a stale observer then subscribes itself to every signal read for the
/// rest of the thread's life (issue #141).
///
/// The displaced observer is *hidden*, not stacked under: nothing read during
/// this run is its, at any depth (issue #3047). The run starts a frame of its
/// own with no suspensions (see [`Suspensions`]), so it tracks even when it
/// runs inside an `untracked` region, and the displaced frame's suspensions
/// come back with it.
pub(super) struct ObserverGuard {
    displaced: Option<ObserverId>,
    displaced_count: u32,
    displaced_frame: u64,
}

impl ObserverGuard {
    /// Make `id` the current observer.
    pub(super) fn push(id: ObserverId) -> Self {
        RUNTIME.with(|rt| {
            let rt = &mut *rt.borrow_mut();
            let s = &mut rt.suspensions;
            s.last_frame += 1;
            ObserverGuard {
                displaced: rt.observer.replace(id),
                displaced_count: std::mem::replace(&mut s.count, 0),
                displaced_frame: std::mem::replace(&mut s.frame, s.last_frame),
            }
        })
    }

    /// Make `id` current for a pass that re-reads its dependencies — an effect
    /// body, or a memo's lazy recompute — which is definitely about to happen.
    ///
    /// Releases the subscriptions the previous such pass took out first, so the
    /// observer ends up subscribed to exactly what it read this time (issue
    /// #171): a dependency it stops reading stops both waking it and holding its
    /// id. "Definitely about to happen" is load-bearing — `run_effect` takes
    /// the body borrow before pushing, so the re-entrant run it skips cannot
    /// strip the subscriptions of the run already in progress beneath it.
    pub(super) fn push_retracking(id: ObserverId) -> Self {
        super::effect::unsubscribe_deps(id);
        Self::push(id)
    }
}

impl Drop for ObserverGuard {
    fn drop(&mut self) {
        let (displaced, count, frame) =
            (self.displaced, self.displaced_count, self.displaced_frame);
        let _ = restore::with_runtime_on_drop("the current observer", |rt| {
            rt.observer = displaced;
            let s = &mut rt.suspensions;
            s.count = count;
            s.frame = frame;
            if !s.deferred.is_empty() {
                s.settle_deferred();
            }
        });
    }
}

/// Hides the current observer until dropped; the shared body of
/// [`untracked`], [`untracked_handler`] and [`suspend_tracking`], and the
/// guard disposal holds over teardown code (issue #494).
///
/// Suspensions may end in any order and outlive the run they were taken in;
/// [`Suspensions`] says how each case is counted. Tracking resumes when the
/// last suspension taken in the run ends, never earlier.
///
/// Observers that *run* during the suspension track normally — a write in it
/// flushes effects synchronously, and code in it may create one; each starts
/// a frame of its own for its run.
pub(in crate::reactive) struct SuspendObservers {
    /// The frame this suspension was taken in.
    frame: u64,
}

impl SuspendObservers {
    pub(in crate::reactive) fn take() -> Self {
        RUNTIME.with(|rt| {
            let s = &mut rt.borrow_mut().suspensions;
            s.count += 1;
            SuspendObservers { frame: s.frame }
        })
    }
}

impl Drop for SuspendObservers {
    fn drop(&mut self) {
        let frame = self.frame;
        let _ = restore::with_runtime_on_drop("the observer a suspension hid", |rt| {
            let s = &mut rt.suspensions;
            if frame == s.frame {
                debug_assert!(s.count > 0, "a suspension released twice");
                s.count = s.count.saturating_sub(1);
            } else if frame < s.frame {
                // Possibly a frame a nested run is hiding: settle it there.
                s.deferred.push(frame);
            }
            // A larger id is a frame that has ended; its count ended with it.
        });
    }
}

/// Run a function without tracking any signal reads.
///
/// Useful for reading signals without creating subscriptions. A read inside
/// `f` subscribes no observer at all — not the effect or memo whose run made
/// the call, and not one whose run that one is nested inside (issue #3047).
///
/// Observers that *run* inside `f` (a write there flushing outside a batch, an
/// effect created there) track their own reads normally.
///
/// # See also
///
/// [`unowned`](super::unowned) suspends the *owner* stack instead of the
/// observer — it changes who owns resources created inside `f`, not who
/// subscribes to signals read inside it. The two are independent, and picking
/// the wrong one fails silently.
pub fn untracked<R>(f: impl FnOnce() -> R) -> R {
    let _suspended = SuspendObservers::take();
    f()
}

/// Run an app's callback with no observer in view (issues #285, #931).
///
/// Call this wherever framework or library code invokes a callback the *app*
/// supplied — an event handler, a hook, an observer, a dismiss handler — from a
/// place that may be inside a running effect. Handlers respond to events;
/// effects respond to signals. Without it, every signal the callback reads is
/// recorded as a dependency of whichever effect happened to make the call, and
/// that effect re-runs on the app's writes to state it never read itself.
///
/// Every app-callback type's `invoke` ([`Callback`](crate::Callback),
/// [`ValueCallback`](crate::ValueCallback), [`InputCallback`](crate::InputCallback),
/// [`FileDropCallback`](crate::FileDropCallback),
/// [`ScrollCallback`](crate::events::ScrollCallback)) goes through here, and so
/// do the selection and selection-sync callbacks, the keyboard and paste
/// interceptors, the dismiss stack,
/// [`Drag::cancel`](crate::events::Drag::cancel)'s `on_cancel`, the child
/// observers ([`on_child_inserted`](crate::dom::on_child_inserted) — which every
/// `for` reconcile reaches from inside its effect) and the rich-text editor's
/// `EditorHandle` hooks.
///
/// It does what [`untracked`] does. It is a separate name because it marks a
/// different decision — "this is app code, never a dependency" rather than "I
/// read this on purpose without depending on it" — and it predates #3047, when
/// `untracked` hid only the innermost observer and this one hid them all
/// (#932).
///
/// Only tracking changes. Batching, the owner stack and effect flushing are
/// untouched: a callback's writes flush exactly when a bare call's would, and a
/// signal it creates belongs to whatever owner is ambient — combine with
/// [`Owner::run`](super::Owner::run) or [`unowned`](super::unowned) for that.
///
/// Do **not** use it around code that is the caller's own synchronous work,
/// such as the closure handed to `EditorHandle::update`: an effect that reads a
/// signal there does mean to depend on it.
pub fn untracked_handler<R>(f: impl FnOnce() -> R) -> R {
    let _suspended = SuspendObservers::take();
    f()
}

/// [`untracked_handler`] as a guard: tracking is suspended from this call
/// until the returned value is dropped (issue #943).
///
/// For library code that runs app-supplied code under a borrow it holds in a
/// guard of its own, where a closure cannot span the region — the rich-text
/// editor's `EditorHandle` holds one beside every borrow of its core, so the
/// plugin methods the handle itself runs (`apply`, `decorations`,
/// `handle_paste`, a plugin's command) are untracked. Everything
/// [`untracked_handler`] says applies, including its warning: the caller's
/// own synchronous code must run with the guard dropped — and so plugin code
/// the caller's code calls in that window (an `update` closure calling
/// `state.apply(..)`) is tracked, as the caller's.
///
/// Guards may be dropped in any order: tracking resumes when the last one
/// taken in the run is dropped. A guard dropped after the run that took it has
/// finished resumes nothing, since that run's observer is no longer current.
/// The guard is `!Send` and `!Sync`: the tracking state it releases is
/// thread-local.
///
/// ```compile_fail
/// fn assert_send<T: Send>(_: T) {}
/// assert_send(rinch_core::reactive::suspend_tracking());
/// ```
#[must_use = "tracking resumes as soon as the guard is dropped"]
pub struct TrackingSuspended {
    _suspended: SuspendObservers,
    /// `!Send` + `!Sync`: dropped on another thread it would restore this
    /// thread's observer onto that thread's runtime.
    _not_send: std::marker::PhantomData<*const ()>,
}

impl std::fmt::Debug for TrackingSuspended {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TrackingSuspended")
    }
}

/// Suspend dependency tracking until the returned guard is dropped; see
/// [`TrackingSuspended`].
pub fn suspend_tracking() -> TrackingSuspended {
    TrackingSuspended {
        _suspended: SuspendObservers::take(),
        _not_send: std::marker::PhantomData,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reactive::{Effect, Memo, Signal};
    use std::cell::Cell;
    use std::rc::Rc;

    /// A panic inside an effect must not strand its `ObserverId` as the
    /// current observer (issue #141).
    ///
    /// The slot is what `Signal::track` reads to decide who subscribes, so a
    /// stranded id silently subscribes itself to *every* signal read for the
    /// rest of the thread's life — and that observer's slot is gone, so the
    /// symptom is unbounded queue churn far from the panic that caused it.
    #[test]
    fn a_panicking_effect_does_not_strand_its_observer() {
        assert_eq!(current_observer(), None);

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            Effect::new(|| panic!("boom"));
        }));
        assert!(result.is_err(), "the effect body must have panicked");
        assert_eq!(
            current_observer(),
            None,
            "the current observer must unwind with the panic"
        );

        // Positive control: the runtime is still usable afterwards, and a
        // signal read at top level (outside any effect) does not wake anyone.
        // With a stranded observer this signal would acquire a subscriber whose
        // slot is gone, and every later `set` would queue it forever.
        let sig = Signal::new(0);
        let runs = Rc::new(Cell::new(0));
        let r = runs.clone();
        let _e = Effect::new(move || {
            sig.get();
            r.set(r.get() + 1);
        });
        assert_eq!(runs.get(), 1, "the new effect runs once at creation");

        sig.get(); // top-level read: must not subscribe anything
        sig.set(1);
        assert_eq!(
            runs.get(),
            2,
            "exactly the one live subscriber re-runs, no more"
        );
    }

    /// The same guarantee for a memo, whose user computation runs lazily in
    /// `Memo::get` rather than in its dirty-marker effect.
    #[test]
    fn a_panicking_memo_computation_does_not_strand_its_observer() {
        let memo = Memo::new(|| -> i32 { panic!("boom") });
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| memo.get()));
        assert!(result.is_err(), "the memo computation must have panicked");
        assert_eq!(
            current_observer(),
            None,
            "the current observer must unwind with the panic"
        );
    }

    /// A panic in an effect nested inside another's run puts the *outer* one
    /// back, so the outer run's later reads are still its own.
    #[test]
    fn a_panic_in_a_nested_run_restores_the_outer_observer() {
        let later = Signal::new(0);
        let runs = Rc::new(Cell::new(0u32));
        let _outer = Effect::new({
            let runs = runs.clone();
            move || {
                runs.set(runs.get() + 1);
                let caught = std::panic::catch_unwind(|| {
                    Effect::new(|| panic!("boom"));
                });
                assert!(caught.is_err());
                later.get();
            }
        });
        later.set(1);
        assert_eq!(runs.get(), 2, "the read after the caught panic is tracked");
    }

    /// Suspensions dropped out of order (two editor guards, say) keep the
    /// observer hidden until the last one goes, and then restore it.
    #[test]
    fn suspensions_dropped_out_of_order_stay_suspended_until_the_last() {
        let between = Signal::new(0);
        let later = Signal::new(0);
        let runs = Rc::new(Cell::new(0u32));
        let _e = Effect::new({
            let runs = runs.clone();
            move || {
                runs.set(runs.get() + 1);
                let first = suspend_tracking();
                let second = suspend_tracking();
                drop(first);
                between.get(); // `second` is still alive: untracked
                drop(second);
                later.get();
            }
        });
        between.set(1);
        assert_eq!(
            runs.get(),
            1,
            "a read while one guard is alive is untracked"
        );
        later.set(1);
        assert_eq!(runs.get(), 2, "the read after both guards is tracked");
    }

    /// A guard that outlives the run it was taken in does not bring that
    /// run's observer back when it is dropped.
    #[test]
    fn a_guard_dropped_after_its_run_restores_nothing() {
        let stash: Rc<std::cell::RefCell<Option<TrackingSuspended>>> = Rc::default();
        let read_at_top = Signal::new(0);
        let runs = Rc::new(Cell::new(0u32));
        let _e = Effect::new({
            let (stash, runs) = (stash.clone(), runs.clone());
            move || {
                runs.set(runs.get() + 1);
                *stash.borrow_mut() = Some(suspend_tracking());
            }
        });
        drop(stash.borrow_mut().take());
        assert_eq!(current_observer(), None, "no observer outside every run");
        read_at_top.get();
        read_at_top.set(1);
        assert_eq!(runs.get(), 1, "a top-level read subscribed nobody");
    }

    /// A guard released inside a run nested in the one that took it leaves
    /// the nested run's tracking alone, and the outer run tracks again once
    /// the nested one returns.
    #[test]
    fn a_guard_released_in_a_nested_run_resumes_its_own_run() {
        let stash: Rc<std::cell::RefCell<Option<TrackingSuspended>>> = Rc::default();
        let (wake, inner_dep, outer_dep) = (Signal::new(0), Signal::new(0), Signal::new(0));
        let (inner_runs, outer_runs) = (Rc::new(Cell::new(0u32)), Rc::new(Cell::new(0u32)));
        let _inner = Effect::new({
            let (stash, inner_runs) = (stash.clone(), inner_runs.clone());
            move || {
                inner_runs.set(inner_runs.get() + 1);
                wake.get();
                drop(stash.borrow_mut().take());
                inner_dep.get();
            }
        });
        let _outer = Effect::new({
            let (stash, outer_runs) = (stash.clone(), outer_runs.clone());
            move || {
                outer_runs.set(outer_runs.get() + 1);
                if outer_runs.get() > 1 {
                    return;
                }
                *stash.borrow_mut() = Some(suspend_tracking());
                // Outside a batch: the inner effect runs right here, nested,
                // and drops the guard.
                wake.set(1);
                outer_dep.get();
            }
        });
        assert_eq!(
            inner_runs.get(),
            2,
            "the inner effect ran nested in the outer"
        );
        assert!(stash.borrow().is_none(), "the nested run dropped the guard");

        inner_dep.set(1);
        assert_eq!(
            (inner_runs.get(), outer_runs.get()),
            (3, 1),
            "the nested run's read after the release is its own"
        );
        outer_dep.set(1);
        assert_eq!(
            outer_runs.get(),
            2,
            "the outer run tracked again after the nested one"
        );
    }
}
