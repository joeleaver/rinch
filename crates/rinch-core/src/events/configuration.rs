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
/// inside a render closure, once per repaint — than a genuine second listener,
/// and the last-write-wins slot at least keeps that bug bounded.
///
/// **It runs on the main thread.** That is the whole reason the shell defers
/// this to its own loop body rather than calling straight out of the
/// `android_activity` callback: a handler that writes a [`Signal`] must be on
/// the thread that owns the signal store, or `Signal::set` panics and tells you
/// to use `send`. Because it is on the main thread, a handler may write signals
/// directly, and the repaint that follows is the ordinary reactive one.
///
/// [`Signal`]: crate::Signal
pub fn set_configuration_change_handler<F>(cb: F)
where
    F: Fn() + 'static,
{
    CONFIGURATION_CHANGE.with(|slot| {
        *slot.borrow_mut() = Some(Rc::new(cb));
    });
}

/// Forget the handler registered by [`set_configuration_change_handler`].
pub fn clear_configuration_change_handler() {
    CONFIGURATION_CHANGE.with(|slot| {
        *slot.borrow_mut() = None;
    });
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
/// out from under the tree very plausibly does.
pub fn dispatch_configuration_change() {
    let handler = CONFIGURATION_CHANGE.with(|slot| slot.borrow().clone());
    if let Some(handler) = handler {
        handler();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

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
}
