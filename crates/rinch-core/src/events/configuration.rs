//! The platform telling the app that something it read at mount is now stale.

use std::cell::RefCell;
use std::rc::Rc;

/// A callback the app registers to hear about configuration changes. Takes
/// nothing and returns nothing, because there is nothing honest to hand it.
///
/// The temptation is to pass "what changed" — a bitmask, or the new night-mode
/// flag, or a `Configuration` struct — and every version of that is a lie
/// waiting to be believed. Android's own `onConfigurationChanged` hands over a
/// whole `Configuration` and the framework's advice has always been to re-read
/// the resources rather than diff it, for the good reason that the fields an
/// app cares about are rarely the fields the platform bothered to change: a
/// theme flip also moves the wallpaper colours, a font-scale change also moves
/// every measured height, and an OEM skin will raise one event for a pair of
/// changes it decided were related. A payload would also fix this hook to
/// Android's idea of a configuration, and this type lives in `rinch-core`
/// precisely so it does not name a platform.
///
/// So the contract is the weakest one that is always true: *something you were
/// told about the device has changed; ask again.* The asking is done by the
/// same calls the app used at mount, which are already the calls that know how
/// to fail.
pub type ConfigurationChangeHandler = Rc<dyn Fn()>;

thread_local! {
    static CONFIGURATION_CHANGE: RefCell<Option<ConfigurationChangeHandler>> = const { RefCell::new(None) };
}

/// Register the handler that runs when the platform's configuration changes.
///
/// **Why this lives in `rinch-core` and not in `rinch-android`.** The event
/// itself has exactly one producer today — the Android shell's
/// `MainEvent::ConfigChanged` arm — so `rinch-android` looks like its natural
/// home right up until you write the call site. `rinch-android` is a
/// dependency an app declares under
/// `[target.'cfg(target_os = "android")'.dependencies]` and whose every module
/// is `#[cfg(target_os = "android")]`, so a hook there is *absent* on the
/// desktop rather than inert on it, and the app pays for that with a
/// `#[cfg]`-and-stub pair around a line that has no business knowing which
/// platform it is on. `rinch-core` is compiled unconditionally on every target
/// rinch supports, which makes the desktop's no-op free: the handler can be
/// registered anywhere, and on a platform with no shell to dispatch it, it is
/// simply never called. That is the same shape [`set_keyboard_interceptor`] has
/// and for the same reason — core owns the slot, the platform shell decides
/// whether anything ever arrives in it.
///
/// [`set_keyboard_interceptor`]: crate::events::set_keyboard_interceptor
///
/// **One handler, replaced rather than added to**, again following
/// `set_keyboard_interceptor`. A list would need a registration handle to
/// remove entries from, and the thing that actually wants this is an app's
/// single "re-read the platform" routine at the root of its tree. A second
/// caller here is much more likely to be a bug — a component registering from
/// inside a render closure, once per repaint — than a genuine second listener.
///
/// **It runs on the main thread.** That is the whole reason the shell defers
/// this to its own loop body rather than calling straight out of the
/// `android_activity` callback: a handler that writes a [`Signal`] must be on
/// the thread that owns the signal store, or `Signal::set` panics and tells you
/// to use `send`. Because it is on the main thread, a handler may write signals
/// directly, and the repaint that follows is the ordinary reactive one.
///
/// [`Signal`]: crate::Signal
///
/// # Released on unmount (issue #183)
///
/// Registering from inside a render ties the handler to the ambient scope, so
/// disposing that scope releases it. That is not housekeeping: since #141 PR4 a
/// scope **owns** the signals created while it was the ambient owner and
/// disposing it frees them, so a handler that outlives its component reads
/// freed state — and a read of a freed [`Signal`] *panics*, where a write is a
/// warn-once no-op. Registering outside any render — from `main`, a timer, a
/// detached callback — has no owner and keeps app lifetime, which is the
/// pre-#141 default and the commonest registration this API has.
///
/// **This hook makes the stale-handler window the whole life of the app.** A
/// configuration change arrives long after mount *by nature* — a night-mode
/// flip at sunset, a rotation, a font-scale change — so "the component that
/// registered has unmounted since" is the ordinary case here, not the exotic
/// one other registries have to reach for.
///
/// # Why the thread-scoped helper, when the sibling registries are doc-scoped
///
/// [`set_keyboard_interceptor`], [`set_paste_interceptor`],
/// [`set_selection_callback`] and [`set_selection_sync_callback`] all use
/// [`install_doc_scoped_slot`]: one slot per document plus a thread-global
/// fallback, so two `RinchContext`s on one thread do not share one interceptor
/// (#134). This registry deliberately uses the plain [`install_scoped_slot`],
/// and is **its first production caller** — so if you are here because the
/// asymmetry looked like an oversight, it is not one, and this section is the
/// reason.
///
/// **"All four siblings use the doc-keyed one" is the trap, not the argument.**
/// It is the obvious justification, it is what the triage for this change
/// recommended, and following it would have *introduced* a defect rather than
/// avoided one. Read the next paragraph before changing this line.
///
/// The reason is mechanical rather than stylistic. A doc-keyed slot **resolves
/// on the document that is dispatching right now**: [`read_doc_scoped_slot`]
/// reads [`current_dispatching_doc`] and falls back to the ownerless entry only
/// when that is `None`. A configuration change has no dispatching document — it
/// is a process-level event dispatched from the Android shell's loop body,
/// beside `drain_lifecycle`, and the only production [`push_dispatching_doc`]
/// in the workspace is `RinchApp`'s event-dispatch entry point, which this is
/// not inside. So the key at dispatch is always `None`, only the ownerless
/// entry would ever be read, and a handler registered from inside an event
/// handler — keyed to *that* document — would never fire again. Doc-scoping
/// here does not merely fail to help; it turns last-write-wins into
/// silently-never-runs.
/// `a_handler_registered_during_a_documents_dispatch_still_fires_from_the_platform_loop`
/// is the fixture that holds that line.
///
/// **What the thread-scoped slot costs, stated rather than implied:** two
/// `RinchContext`s on one thread share one handler, last registration wins. If
/// that ever needs fixing, the answer is a **list dispatched to every
/// registrant**, not a key — there is no dispatching document for a key to
/// resolve. Having the shell push a document key around the dispatch would only
/// pick one document for a process-level event and silently disable the others,
/// which is the same failure relocated.
///
/// # Why not the dispatch-checked template
///
/// The third shape in the codebase — keep the [`Owner`] beside the callback and
/// test `is_alive` when the event arrives — is what
/// [`park_main_callback`](crate::main_thread::park_main_callback), `rinch-ws`'s
/// handler registry, the menu registry and `rinch-android`'s sensor / location
/// / lifecycle registries use. `rinch-android`'s `scoped.rs` states its three
/// criteria, and this registry meets none of them:
///
/// 1. *Written repeatedly from a live component.* This one is an app's single
///    re-read routine, written once — the same premise the one-slot-not-a-list
///    decision above rests on.
/// 2. *The callback allocates state that should belong to its component.* A
///    handler here writes signals that already exist; it does not create them.
/// 3. *The registry is drained once a frame*, which makes a liveness check free
///    and **prompt**. This one is not drained at all — it fires on a
///    configuration change, which is rare and may never come again, so a
///    liveness check would leave a dead component's handler in the slot
///    indefinitely. The cleanup-tied template releases it at unmount.
///
/// The residual is the one every [`install_scoped_slot`] caller carries: one
/// `on_cleanup` per registration, so a handler re-registered from a re-running
/// effect grows that scope's cleanup vec. That is documented at the helper, and
/// the fix named there (one cleanup per `(slot, owner)`) closes it for every
/// caller at once rather than specially here.
///
/// [`set_paste_interceptor`]: crate::events::set_paste_interceptor
/// [`set_selection_callback`]: crate::events::set_selection_callback
/// [`set_selection_sync_callback`]: crate::events::set_selection_sync_callback
/// [`install_doc_scoped_slot`]: crate::reactive::install_doc_scoped_slot
/// [`install_scoped_slot`]: crate::reactive::install_scoped_slot
/// [`read_doc_scoped_slot`]: crate::reactive::read_doc_scoped_slot
/// [`current_dispatching_doc`]: crate::context::current_dispatching_doc
/// [`push_dispatching_doc`]: crate::context::push_dispatching_doc
/// [`Owner`]: crate::reactive::Owner
pub fn set_configuration_change_handler<F>(cb: F)
where
    F: Fn() + 'static,
{
    crate::reactive::install_scoped_slot(&CONFIGURATION_CHANGE, Rc::new(cb) as Rc<dyn Fn()>);
}

/// Forget the handler registered by [`set_configuration_change_handler`].
///
/// Goes through [`clear_scoped_slot`](crate::reactive::clear_scoped_slot) so the
/// removed handler is dropped **after** the borrow ends: it is user code whose
/// `Drop` may re-enter this registry, which under the `borrow_mut` would panic.
/// Any cleanup the registering scope still holds is left in place and becomes a
/// no-op, because its `Weak` can no longer upgrade.
pub fn clear_configuration_change_handler() {
    crate::reactive::clear_scoped_slot(&CONFIGURATION_CHANGE);
}

/// Run the registered handler, if there is one. Called by the platform shell;
/// an app has no reason to call it, and nothing breaks if it does.
///
/// The `Rc` is cloned out of the slot before the call and the borrow is
/// dropped, which is not tidiness — it is what makes a handler that calls
/// [`set_configuration_change_handler`] on itself (or clears it, or is dropped
/// as a consequence of what it wrote) a re-registration rather than a
/// `BorrowMutError`. `dispatch_keyboard_event` holds its borrow across the
/// call and gets away with it because a keypress interceptor has no reason to
/// re-register mid-keypress; a configuration handler that swaps a whole theme
/// out from under the tree very plausibly does. That rule lives in
/// [`read_scoped_slot`](crate::reactive::read_scoped_slot) rather than being
/// paraphrased here.
pub fn dispatch_configuration_change() {
    if let Some(handler) = crate::reactive::read_scoped_slot(&CONFIGURATION_CHANGE) {
        handler();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    use crate::reactive::{Scope, Signal};

    /// The desktop's whole contract: registering is legal everywhere, and a
    /// dispatch that never comes is not an error.
    #[test]
    fn a_handler_that_is_never_dispatched_is_not_an_error() {
        clear_configuration_change_handler();
        set_configuration_change_handler(|| unreachable!("nothing dispatched here"));
        clear_configuration_change_handler();
    }

    #[test]
    fn dispatch_runs_the_handler_and_a_cleared_slot_is_a_no_op() {
        clear_configuration_change_handler();
        // No handler at all is the state a desktop build is in for its whole
        // life, so it had better be silent rather than merely survivable.
        dispatch_configuration_change();

        let runs = Rc::new(Cell::new(0u32));
        let seen = Rc::clone(&runs);
        set_configuration_change_handler(move || seen.set(seen.get() + 1));
        dispatch_configuration_change();
        dispatch_configuration_change();
        assert_eq!(runs.get(), 2, "every dispatch reaches the handler");

        clear_configuration_change_handler();
        dispatch_configuration_change();
        assert_eq!(runs.get(), 2, "and none does after it is cleared");
    }

    /// The reason `dispatch` clones the `Rc` out before calling. A handler that
    /// re-registers — an app swapping its theme handler as the theme changes —
    /// would panic with a `BorrowMutError` if the slot were still borrowed.
    #[test]
    fn a_handler_may_replace_itself_from_inside_the_dispatch() {
        clear_configuration_change_handler();
        let runs = Rc::new(Cell::new(0u32));

        let first = Rc::clone(&runs);
        set_configuration_change_handler(move || {
            first.set(first.get() + 1);
            let second = Rc::clone(&first);
            set_configuration_change_handler(move || second.set(second.get() + 10));
        });

        dispatch_configuration_change();
        assert_eq!(runs.get(), 1, "the first handler ran");
        dispatch_configuration_change();
        assert_eq!(runs.get(), 11, "and the one it installed ran next");

        clear_configuration_change_handler();
    }

    /// The clear half of the same re-entrancy rule. The test above covers a
    /// handler that *replaces* itself; one that **removes** itself — a one-shot
    /// "re-read once and stop listening" — goes through the other setter, which
    /// has its own borrow to get wrong.
    #[test]
    fn a_handler_may_clear_itself_from_inside_the_dispatch() {
        clear_configuration_change_handler();
        let runs = Rc::new(Cell::new(0u32));

        let seen = Rc::clone(&runs);
        set_configuration_change_handler(move || {
            seen.set(seen.get() + 1);
            clear_configuration_change_handler();
        });

        dispatch_configuration_change();
        assert_eq!(runs.get(), 1, "it ran once");
        dispatch_configuration_change();
        assert_eq!(runs.get(), 1, "and removed itself while it was running");
    }

    /// **Released on unmount** (issue #183), which is this hook's defining
    /// lifetime question rather than an incidental one.
    ///
    /// A configuration change arrives long after mount *by nature* — a
    /// night-mode flip at sunset, a rotation, a font-scale change — so "the
    /// component that registered has unmounted since" is the ordinary case
    /// here, not the exotic one other registries have to reach for.
    #[test]
    fn a_handler_registered_in_a_render_is_released_when_its_component_unmounts() {
        clear_configuration_change_handler();
        let runs = Rc::new(Cell::new(0u32));

        let scope = Scope::new();
        let seen = Rc::clone(&runs);
        scope.run(|| set_configuration_change_handler(move || seen.set(seen.get() + 1)));

        // Fail-first inside the fixture: prove it fires while the component is
        // mounted, so "it did not fire" below cannot pass for the wrong reason.
        dispatch_configuration_change();
        assert_eq!(runs.get(), 1, "precondition: it runs while mounted");

        scope.dispose();
        dispatch_configuration_change();
        assert_eq!(runs.get(), 1, "and never again once its component is gone");
    }

    /// The same defect at the point where it actually bites, rather than where
    /// it is convenient to observe.
    ///
    /// The whole purpose of this handler is to re-read the platform *into
    /// signals*, so the realistic body reads a signal its own component owns.
    /// Since #141 PR4, disposing that component frees the signal and a read
    /// **panics** — so an un-released handler does not merely go stale, it
    /// crashes the app on the next night-mode flip. This fixture fails by
    /// panicking in `panic_read_freed`, which is the device failure itself.
    #[test]
    fn a_released_handler_never_reads_the_signal_its_component_owned() {
        clear_configuration_change_handler();

        let scope = Scope::new();
        scope.run(|| {
            let dark = Signal::new(false);
            set_configuration_change_handler(move || {
                // Reaching this line after `scope.dispose()` is the bug: a read
                // of a freed signal panics rather than answering staleness.
                let _ = dark.get();
            });
        });

        scope.dispose();
        dispatch_configuration_change();
        clear_configuration_change_handler();
    }

    /// **Rule 1: ownerless registration keeps app lifetime.** An app that
    /// registers from `main` — before there is a tree at all, which is where
    /// the single "re-read the platform" routine most naturally goes — must not
    /// have its handler collected because some unrelated component unmounted.
    ///
    /// This is the assertion that stops the fix overshooting: a registration
    /// path that required an owner, or a cleanup that cleared the slot
    /// unconditionally, would satisfy every test above and break the commonest
    /// registration there is.
    #[test]
    fn a_handler_registered_outside_any_render_keeps_app_lifetime() {
        clear_configuration_change_handler();
        let runs = Rc::new(Cell::new(0u32));

        let seen = Rc::clone(&runs);
        set_configuration_change_handler(move || seen.set(seen.get() + 1));

        // An unrelated component mounts and unmounts in between.
        let unrelated = Scope::new();
        unrelated.run(|| {});
        unrelated.dispose();

        dispatch_configuration_change();
        assert_eq!(
            runs.get(),
            1,
            "a handler with no owner lives for the life of the app"
        );
        clear_configuration_change_handler();
    }

    /// **Rule 2: an earlier unmount must not clobber a later registration.**
    ///
    /// Two components register in turn and the *first* unmounts. The slot holds
    /// the second's handler, and the first's release must leave it alone —
    /// which is exactly what the obvious wrong fix, an
    /// `on_cleanup(clear_configuration_change_handler)` beside a bare write,
    /// gets wrong.
    ///
    /// Worth being precise about what carries this here, because the general
    /// helper has two guards and only one of them is reachable through this
    /// API: the setter owns the `Rc` it wraps and drops the displaced one, so a
    /// caller can never retain the value it installed. The first scope's `Weak`
    /// therefore fails to upgrade and its cleanup returns early — [`Rc::ptr_eq`]
    /// is belt-and-braces for this registry rather than the thing under test,
    /// and `scoped.rs` owns the fixture that exercises it directly.
    #[test]
    fn an_earlier_unmount_does_not_clobber_a_later_registration() {
        clear_configuration_change_handler();
        let runs = Rc::new(Cell::new(0u32));

        let first = Scope::new();
        let a = Rc::clone(&runs);
        first.run(|| set_configuration_change_handler(move || a.set(a.get() + 1)));

        let second = Scope::new();
        let b = Rc::clone(&runs);
        second.run(|| set_configuration_change_handler(move || b.set(b.get() + 10)));

        first.dispose();
        dispatch_configuration_change();
        assert_eq!(
            runs.get(),
            10,
            "the second component's handler is still installed, and it is the \
             one that ran"
        );

        second.dispose();
        dispatch_configuration_change();
        assert_eq!(runs.get(), 10, "and the owner of the slot does reclaim it");
    }

    /// **The fixture that says why the slot is thread-scoped and not
    /// doc-scoped** — the one decision here a reader is most likely to
    /// "correct" back, since the sibling registries in this module all use the
    /// doc-keyed helper.
    ///
    /// Registering happens inside a document's dispatch: an `onclick` that
    /// starts listening for theme changes is entirely ordinary. Dispatching
    /// happens from the platform loop body, where **no** document is
    /// dispatching — a configuration change is a process-level event, and the
    /// only production `push_dispatching_doc` in the workspace is `RinchApp`'s
    /// event entry point, which the shell's `ConfigChanged` drain is not inside.
    ///
    /// A doc-keyed slot would file that registration under document 1 and then
    /// resolve the dispatch against the ownerless entry, so the handler would
    /// never run again — silently, with nothing to observe but an app that
    /// quietly stopped following the system theme.
    #[test]
    fn a_handler_registered_during_a_documents_dispatch_still_fires_from_the_platform_loop() {
        use crate::context::{current_dispatching_doc, push_dispatching_doc};

        clear_configuration_change_handler();
        let runs = Rc::new(Cell::new(0u32));

        let seen = Rc::clone(&runs);
        {
            let _dispatching = push_dispatching_doc(1);
            assert_eq!(
                current_dispatching_doc(),
                Some(1),
                "precondition: the registration really is made under a document"
            );
            set_configuration_change_handler(move || seen.set(seen.get() + 1));
        }

        assert_eq!(
            current_dispatching_doc(),
            None,
            "precondition: the platform loop dispatches outside any document"
        );
        dispatch_configuration_change();
        assert_eq!(
            runs.get(),
            1,
            "a handler registered under a document must still hear a \
             process-level event dispatched outside one"
        );
        clear_configuration_change_handler();
    }
}
