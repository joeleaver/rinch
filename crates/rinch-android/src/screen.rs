//! Holding the display awake while the app is the thing being looked at.
//!
//! The name is deliberately about the *screen* and not about *waking*. A phone
//! has more than one thing in it that can be asleep — the panel, the CPU, a
//! blocked thread — and a module called `wake` in a UI crate would be a magnet
//! for all three. This one keeps the display lit and does nothing else.
//!
//! # Why the window flag and not a wake lock
//!
//! There are two ways to stop the screen going off and they fail differently.
//!
//! `PowerManager.WakeLock` is a handle held by the *process*. It needs
//! `android.permission.WAKE_LOCK` in the manifest, it keeps its grip while the
//! app is in the background, and releasing it is the caller's job — so every
//! path out of the state that wanted it, including the ones nobody thought of,
//! is a path that can leak it. What a leaked screen lock looks like to the
//! person holding the phone is a battery that was full at the start of the
//! evening and is not at the end, with nothing on screen to explain it.
//!
//! `WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON` is a property of the
//! *window*. It needs no permission, and the system honours it only while that
//! window is the one being shown — background the app and the display timeout
//! comes straight back. Note what that does and does not undo: the *effect* is
//! suspended, the *flag* is not. It stays in the window's `LayoutParams`, so
//! returning to the app holds the screen awake again, and nothing clears it
//! before the activity dies. The worst leak it can produce is therefore a
//! screen that does not sleep for the rest of the session — bounded by the
//! activity, and costing nothing while the user is elsewhere. That asymmetry is
//! the whole argument: a keep-awake API is going to be misused eventually, and
//! this is the one whose misuse cannot follow the user out of the app.
//!
//! An app that genuinely needs the CPU to keep running with the screen off
//! wants neither of these — it wants a foreground service, which is a manifest
//! and a lifecycle rather than a function call, and is not something this crate
//! can wrap.
//!
//! # A guard, because the flag is a device resource and something must own it
//!
//! [`keep_screen_on`] hands back a [`KeepScreenOn`] guard rather than taking a
//! bool, for the same reason `sensors::start` and `location::start` power the
//! hardware down when the component that registered them dies: a device
//! resource with no owner outlives whoever wanted it. The natural place to ask
//! for the flag is inside a component, and since #141 PR4 a scope owns what was
//! created under it and disposing frees it — so the effect that would have
//! recomputed "should the screen be on" dies at unmount, while a flag set by a
//! bare `keep_screen_on(true)` would not. The code that knew when to turn it
//! off is already gone, and the display never sleeps again for the life of the
//! process.
//!
//! The guard is the strongest form of the rule this module's earlier revision
//! asked of its callers — *a caller that has to remember whether it is
//! currently holding the flag is a caller that will one day forget*. With a
//! guard the caller does not remember, it **holds**: release is what `Drop`
//! means, and forgetting to release stops being a category of bug that exists.
//! Holders are refcounted — the flag is written on the 0→1 edge and cleared on
//! the 1→0 edge — so two components wanting the screen awake compose instead
//! of the first release clearing the flag out from under the second. The
//! idempotence underneath (`addFlags` is an or, `clearFlags` an and-not) still
//! does real work: it is what makes those edges cheap and safe to re-assert.
//!
//! Most callers want [`keep_screen_on_while`], the reactive form: it holds a
//! guard while `enabled()` answers `true` and releases it when the component
//! that asked unmounts, whichever comes first.
//!
//! # Why neither `scoped.rs` template applies
//!
//! Everything else in this crate that ties a platform service to a component
//! goes through `scoped.rs`, whose header names two shapes — cleanup-tied and
//! dispatch-checked. Both turn on a callback that is *dispatched*: the
//! dispatch-checked one this crate's registries use notices a dead owner from
//! the per-frame drain that visits every entry. A window flag is written once
//! and never dispatched again — no drain visits it, so a dead owner would
//! never be noticed by anything. The guard supplies liveness **structurally**
//! instead: the drop is the notice. That is a third option the `scoped.rs`
//! framing does not cover, and [`keep_screen_on_while`]'s effect is what
//! reintroduces an owner for the reactive case — the effect is scope-owned,
//! disposal drops its closure, and the closure is what holds the guard.

use std::cell::Cell;
use std::marker::PhantomData;

use rinch_core::reactive::Effect;

#[cfg(target_os = "android")]
use crate::bridge;

thread_local! {
    /// How many [`KeepScreenOn`] guards are alive. `thread_local!` like every
    /// registry in this crate — the guard is `!Send`, so a release always
    /// decrements the count its acquire incremented — and tests isolate for
    /// free.
    static HOLDERS: Cell<usize> = const { Cell::new(0) };
}

/// Holds the display awake while it is alive.
///
/// Refcounted: the window flag is set when the first guard is acquired and
/// cleared when the last one drops, so several holders compose — a video
/// player and a long upload can each hold one without either's release
/// darkening the other's screen. Intermediate acquires and releases touch no
/// platform state at all.
///
/// `!Send + !Sync`: the count is thread-local, and every caller in a rinch app
/// is on the main thread anyway. For "held while some state is true", reach
/// for [`keep_screen_on_while`] instead of storing one of these by hand.
#[must_use = "the display is held awake only while this guard is alive — dropping it immediately releases the hold"]
pub struct KeepScreenOn {
    /// `*const ()` opts out of `Send`/`Sync`, tying the guard to the thread
    /// whose count it incremented.
    _thread_bound: PhantomData<*const ()>,
}

/// Keep the display awake until the returned guard drops.
///
/// Fire and forget on the way down to the platform: the flag write is posted
/// to the Android UI thread (see `RinchActivity.setKeepScreenOn`, and the note
/// there on why a window flag written from the native frame thread is a race
/// rather than a bug you can see), so it has usually not taken effect by the
/// time this returns. There is nothing to wait for — the next frame the system
/// composes is drawn under the new flag.
pub fn keep_screen_on() -> KeepScreenOn {
    let holders = HOLDERS.with(|h| {
        let n = h.get() + 1;
        h.set(n);
        n
    });
    if holders == 1 {
        apply_keep_screen_on(true);
    }
    KeepScreenOn {
        _thread_bound: PhantomData,
    }
}

impl KeepScreenOn {
    /// Hold for the life of the process — the documented opt-out, for a kiosk
    /// binary that acquires from `android_main` and genuinely never releases.
    ///
    /// The count never comes back down, so later holders come and go without
    /// their releases ever clearing the flag out from under the leaked hold.
    pub fn leak(self) {
        std::mem::forget(self);
    }
}

impl Drop for KeepScreenOn {
    fn drop(&mut self) {
        let holders = HOLDERS.with(|h| {
            let n = h.get() - 1;
            h.set(n);
            n
        });
        if holders == 0 {
            apply_keep_screen_on(false);
        }
    }
}

/// Hold the display awake while `enabled()` answers `true` — the state mirror,
/// tied to the scope that asked for it.
///
/// Runs `enabled` inside an [`Effect`], so it re-evaluates whenever the
/// signals it reads change: a [`KeepScreenOn`] guard is acquired on the
/// false→true edge, held across re-runs that keep the answer, and dropped on
/// true→false. Called from inside a component, the effect is scope-owned
/// (#141 PR4), so unmounting the component drops the effect's closure — and
/// with it the guard, releasing the hold without the component remembering to.
/// `keep_screen_on_while(|| true)` therefore reads as "while this component is
/// mounted".
///
/// Called with no ambient scope — from `android_main`, a timer, a detached
/// callback — the effect has app lifetime and the mirror answers forever, the
/// pre-#141 default for ownerless registration.
pub fn keep_screen_on_while(enabled: impl Fn() -> bool + 'static) {
    let mut held: Option<KeepScreenOn> = None;
    // The `Effect` handle is deliberately let go: dropping it does not dispose
    // the effect, and disposal — which drops the closure, and so the guard —
    // belongs to the ambient scope.
    let _ = Effect::new(move || {
        if enabled() {
            if held.is_none() {
                held = Some(keep_screen_on());
            }
        } else {
            // Dropping the guard is the release; an already-empty hand is the
            // "still false" re-run and touches nothing.
            held = None;
        }
    });
}

/// The JNI half of the refcount's edges: ask the platform to set or clear
/// `FLAG_KEEP_SCREEN_ON` on the activity's window.
///
/// A failed call is logged and swallowed, the way every other service wrapper
/// in this crate handles one. The screen going off early is a disappointment;
/// a panic on the frame thread because the activity happened to be tearing
/// down is a crash. (`with_activity` itself still panics if the crate was
/// never initialised — that is a wiring bug, not a runtime condition.)
#[cfg(target_os = "android")]
fn apply_keep_screen_on(on: bool) {
    bridge::with_activity(|env, activity| {
        if let Err(e) = env.call_method(
            activity,
            "setKeepScreenOn",
            "(Z)V",
            &[jni::objects::JValue::Bool(on as jni::sys::jboolean)],
        ) {
            log::warn!("setKeepScreenOn({on}) failed: {e}");
        }
    });
}

/// The host twin of the JNI call above. It **records** rather than ignores, so
/// the refcount's edges can be exercised on a machine with no device attached —
/// nothing in this repository builds an APK in CI (issue #183 PR5).
#[cfg(not(target_os = "android"))]
fn apply_keep_screen_on(_on: bool) {
    #[cfg(test)]
    apply_log::record(_on);
}

/// What the host build has been asked to write to the window flag, in order.
/// `thread_local!`, like the count it mirrors, so tests isolate for free.
#[cfg(all(not(target_os = "android"), test))]
mod apply_log {
    use std::cell::RefCell;

    thread_local! {
        static APPLIED: RefCell<Vec<bool>> = const { RefCell::new(Vec::new()) };
    }

    pub(super) fn record(on: bool) {
        APPLIED.with(|a| a.borrow_mut().push(on));
    }

    /// Everything recorded since the last call, clearing the log.
    pub(super) fn take() -> Vec<bool> {
        APPLIED.with(|a| std::mem::take(&mut *a.borrow_mut()))
    }
}

#[cfg(all(not(target_os = "android"), test))]
mod tests {
    use super::*;
    use rinch_core::Signal;
    use rinch_core::reactive::Scope;

    fn no_traffic() -> Vec<bool> {
        Vec::new()
    }

    /// The flag is written on the 0→1 edge and on no other acquire: a second
    /// holder joins an already-lit screen without another platform write.
    #[test]
    fn the_flag_is_set_on_the_first_acquire_and_not_again() {
        let _ = apply_log::take();

        let a = keep_screen_on();
        assert_eq!(apply_log::take(), vec![true], "0→1 must set the flag");

        let b = keep_screen_on();
        assert_eq!(
            apply_log::take(),
            no_traffic(),
            "1→2 must not touch the platform"
        );

        drop(a);
        drop(b);
    }

    /// The clear happens when the *last* holder releases, not on every
    /// release. Deliberately nested: a fixture that only ever goes 0→1→0 sits
    /// on the one value where "clears on 1→0" and "clears on every release"
    /// agree, and can distinguish neither from "clears when the count is 0".
    #[test]
    fn the_flag_clears_when_the_last_holder_releases_not_before() {
        let _ = apply_log::take();

        let a = keep_screen_on();
        let b = keep_screen_on();
        let _ = apply_log::take();

        drop(b);
        assert_eq!(
            apply_log::take(),
            no_traffic(),
            "2→1 must not clear the flag — a holder remains"
        );

        drop(a);
        assert_eq!(apply_log::take(), vec![false], "1→0 must clear the flag");
    }

    /// `leak` converts a holder into a process-lifetime hold: later guards
    /// come and go without their releases ever clearing the flag, and without
    /// their acquires re-writing it.
    #[test]
    fn a_leaked_holder_keeps_the_flag_for_the_life_of_the_process() {
        let _ = apply_log::take();

        keep_screen_on().leak();
        assert_eq!(apply_log::take(), vec![true]);

        // If `leak` released, this acquire would be a fresh 0→1 and write
        // `true` again; if it kept the guard but not the count, the drop below
        // would be a 1→0 and write `false`. Either is visible here.
        let passerby = keep_screen_on();
        drop(passerby);
        assert_eq!(
            apply_log::take(),
            no_traffic(),
            "a leaked hold pins the count above zero for both edges"
        );
    }

    /// The reactive mirror follows its condition: acquire on false→true,
    /// release on true→false, re-acquire if it comes back. Registered here
    /// with no ambient scope, which is also the app-lifetime case — it must
    /// keep answering across as many edges as arrive.
    #[test]
    fn keep_screen_on_while_follows_the_condition() {
        let _ = apply_log::take();

        let want = Signal::new(false);
        keep_screen_on_while(move || want.get());
        assert_eq!(
            apply_log::take(),
            no_traffic(),
            "not asked for yet — the initial run reads false"
        );

        want.set(true);
        assert_eq!(apply_log::take(), vec![true]);

        want.set(false);
        assert_eq!(apply_log::take(), vec![false]);

        want.set(true);
        assert_eq!(apply_log::take(), vec![true]);
    }

    /// A re-run that keeps the condition true holds the same guard rather
    /// than releasing and re-acquiring — no flag traffic, and no window in
    /// which the count dips to zero.
    #[test]
    fn keep_screen_on_while_holds_across_reruns_that_keep_the_answer() {
        let _ = apply_log::take();

        let brightness = Signal::new(1u32);
        keep_screen_on_while(move || brightness.get() > 0);
        assert_eq!(apply_log::take(), vec![true]);

        brightness.set(2); // re-run, same answer
        assert_eq!(
            apply_log::take(),
            no_traffic(),
            "a rerun that keeps the answer must not cycle the flag"
        );
    }

    /// The release the guard exists for (issue #417's blocker): the component
    /// unmounts while holding, and the flag clears — the effect is
    /// scope-owned, disposal drops its closure, and the closure is what holds
    /// the guard.
    #[test]
    fn keep_screen_on_while_releases_when_the_component_unmounts() {
        let _ = apply_log::take();

        let scope = Scope::new();
        scope.run(|| keep_screen_on_while(|| true));
        assert_eq!(apply_log::take(), vec![true]);

        scope.dispose();
        assert_eq!(
            apply_log::take(),
            vec![false],
            "unmounting the component that asked must release the hold"
        );
    }

    /// Unmounting one holder must not darken another's screen: the dying
    /// component's release decrements, the surviving guard keeps the flag —
    /// the refcount composing across the guard and the reactive mirror.
    #[test]
    fn an_unmounting_component_does_not_release_a_siblings_hold() {
        let _ = apply_log::take();

        let survivor = keep_screen_on();
        let scope = Scope::new();
        scope.run(|| keep_screen_on_while(|| true));
        scope.dispose();
        assert_eq!(
            apply_log::take(),
            vec![true],
            "one write for the first acquire, and nothing for the unmount"
        );

        drop(survivor);
        assert_eq!(apply_log::take(), vec![false]);
    }
}
