//! Android activity lifecycle events.
//!
//! Register callbacks for `onPause`, `onResume`, `onStop`, and `onStart`.
//! Callbacks fire on the main thread during the next drain cycle.

use std::rc::Rc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU8, Ordering};

use crate::scoped::ScopedSlot;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum LifecycleState {
    Created = 0,
    Started = 1,
    Resumed = 2,
    Paused = 3,
    Stopped = 4,
}

static PENDING_EVENT: AtomicU8 = AtomicU8::new(0);
static CURRENT_STATE: Mutex<LifecycleState> = Mutex::new(LifecycleState::Resumed);

thread_local! {
    /// Scope-aware (issue #183): a callback registered inside a component stops
    /// firing, and is released, once that component unmounts. Before, nothing
    /// removed these two slots at all — there was no clear function.
    static ON_PAUSE: ScopedSlot<dyn Fn()> = const { ScopedSlot::new() };
    static ON_RESUME: ScopedSlot<dyn Fn()> = const { ScopedSlot::new() };
}

const EVENT_NONE: u8 = 0;
const EVENT_PAUSED: u8 = 1;
const EVENT_RESUMED: u8 = 2;

/// Register a callback that fires when the activity is paused (e.g., user switches apps).
///
/// Replaces any previous one. Registered from inside a component, the callback
/// stops firing when that component unmounts; registered from `android_main` or
/// any other ownerless context, it has app lifetime.
pub fn on_pause(cb: impl Fn() + 'static) {
    ON_PAUSE.with(|slot| slot.install(Rc::new(cb)));
}

/// Register a callback that fires when the activity is resumed (e.g., user returns).
///
/// Same lifetime rules as [`on_pause`].
pub fn on_resume(cb: impl Fn() + 'static) {
    ON_RESUME.with(|slot| slot.install(Rc::new(cb)));
}

/// Whether a pause callback is currently installed.
#[cfg(test)]
fn pause_callback_installed() -> bool {
    ON_PAUSE.with(|slot| slot.is_installed())
}

/// Get the current lifecycle state.
pub fn state() -> LifecycleState {
    *CURRENT_STATE.lock().unwrap()
}

/// Drain lifecycle events and invoke callbacks.
/// Called from `android_runtime.rs` main loop each frame.
pub fn drain_lifecycle() {
    // Release what unmounted components left behind, whether or not a transition
    // is pending. An app that never backgrounds never dispatches these, so
    // pruning only on dispatch would hold a dead callback — and everything it
    // captured — for the life of the process. Two `Weak` upgrades a frame.
    ON_PAUSE.with(|slot| slot.release_if_dead());
    ON_RESUME.with(|slot| slot.release_if_dead());

    let event = PENDING_EVENT.swap(EVENT_NONE, Ordering::Relaxed);
    match event {
        EVENT_PAUSED => {
            *CURRENT_STATE.lock().unwrap() = LifecycleState::Paused;
            ON_PAUSE.with(|slot| slot.dispatch(|cb| cb()));
        }
        EVENT_RESUMED => {
            *CURRENT_STATE.lock().unwrap() = LifecycleState::Resumed;
            ON_RESUME.with(|slot| slot.dispatch(|cb| cb()));
        }
        _ => {}
    }
}

/// Record that the activity was paused. Called from the runtime's event loop
/// when the `android-activity` glue delivers `MainEvent::Pause` (main thread).
pub fn notify_paused() {
    PENDING_EVENT.store(EVENT_PAUSED, Ordering::Relaxed);
}

/// Record that the activity was resumed. Called from the runtime's event loop
/// when the `android-activity` glue delivers `MainEvent::Resume` (main thread).
pub fn notify_resumed() {
    PENDING_EVENT.store(EVENT_RESUMED, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;
    use rinch_core::Signal;
    use rinch_core::reactive::Scope;
    use std::cell::Cell;
    use std::rc::Rc;

    /// A callback registered while a component was rendering must not run once
    /// that component is gone: it captured the component's `Signal`s, disposal
    /// freed them, and a *read* of a freed signal panics (issue #183, #141 PR4).
    #[test]
    fn a_pause_callback_registered_in_a_scope_is_not_invoked_after_the_scope_disposes() {
        let _serial = crate::test_serial();

        let ran = Rc::new(Cell::new(false));
        let flag = ran.clone();
        let scope = Scope::new();
        scope.run(|| on_pause(move || flag.set(true)));

        scope.dispose();
        notify_paused();
        drain_lifecycle();

        assert!(
            !ran.get(),
            "a pause callback registered by a since-disposed scope must not run"
        );
    }

    /// The same for `on_resume`, which has its own slot.
    #[test]
    fn a_resume_callback_registered_in_a_scope_is_not_invoked_after_the_scope_disposes() {
        let _serial = crate::test_serial();

        let ran = Rc::new(Cell::new(false));
        let flag = ran.clone();
        let scope = Scope::new();
        scope.run(|| on_resume(move || flag.set(true)));

        scope.dispose();
        notify_resumed();
        drain_lifecycle();

        assert!(
            !ran.get(),
            "a resume callback registered by a since-disposed scope must not run"
        );
    }

    /// Nothing has ever removed these two slots — there is no clear function —
    /// so an unmounted component's callback is held, with everything it
    /// captured, for the life of the process. Releasing it must actually drop it,
    /// not merely decline to call it.
    #[test]
    fn a_dead_lifecycle_callback_is_released_rather_than_held_forever() {
        struct DropSpy(Rc<Cell<bool>>);
        impl Drop for DropSpy {
            fn drop(&mut self) {
                self.0.set(true);
            }
        }

        let _serial = crate::test_serial();

        let dropped = Rc::new(Cell::new(false));
        let spy = DropSpy(dropped.clone());
        let scope = Scope::new();
        scope.run(|| {
            on_pause(move || {
                let _keep = &spy;
            })
        });

        scope.dispose();
        notify_paused();
        drain_lifecycle();

        assert!(
            dropped.get(),
            "the dead callback must be dropped, releasing what it captured"
        );
        assert!(
            !pause_callback_installed(),
            "the slot must be empty afterwards, or every later pause re-checks it"
        );
    }

    /// Registration from `main`, from startup code or from a detached callback
    /// has no ambient owner and therefore app lifetime — the pre-#141 default,
    /// which the liveness check must not disturb.
    #[test]
    fn a_lifecycle_callback_registered_with_no_ambient_owner_still_runs() {
        let _serial = crate::test_serial();

        let count = Rc::new(Cell::new(0u32));
        let c = count.clone();
        // Deliberately not inside a `Scope::run`.
        on_resume(move || c.set(c.get() + 1));

        notify_resumed();
        drain_lifecycle();
        notify_resumed();
        drain_lifecycle();

        assert_eq!(count.get(), 2, "an ownerless callback keeps app lifetime");
    }

    /// The callback runs with its registering component as the ambient owner, so
    /// whatever it allocates belongs to that component rather than to whatever
    /// the event loop happened to be doing.
    #[test]
    fn a_live_lifecycle_callback_runs_with_its_component_as_ambient_owner() {
        let _serial = crate::test_serial();

        let scope = Scope::new();
        scope.run(|| {
            on_resume(|| {
                let _owned_by_the_component = Signal::new(0u32);
            })
        });

        let before = scope.owned_counts().signals;
        notify_resumed();
        drain_lifecycle();
        let after = scope.owned_counts().signals;

        assert_eq!(
            after,
            before + 1,
            "a signal created inside the callback must be attributed to the \
             scope that registered it"
        );
        scope.dispose();
    }

    /// A callback may re-register from inside its own dispatch — the archetype
    /// is a state machine that swaps its pause handler. Holding the slot's borrow
    /// across the call makes that a `BorrowMutError`.
    #[test]
    fn a_lifecycle_callback_may_reregister_from_inside_its_own_dispatch() {
        let _serial = crate::test_serial();

        let second_ran = Rc::new(Cell::new(false));
        let flag = second_ran.clone();
        on_pause(move || {
            let f = flag.clone();
            on_pause(move || f.set(true));
        });

        notify_paused();
        drain_lifecycle();
        notify_paused();
        drain_lifecycle();

        assert!(
            second_ran.get(),
            "the replacement registered from inside the dispatch must be installed"
        );
    }
}
