//! Whether the code running now runs because of a deliberate user action: a
//! key going down, or a pointer button or contact going down or up, while
//! [`RinchApp::handle_event`] dispatches it (#2501).
//!
//! Some requests are honoured only as the answer to a user action. The
//! xdg-activation token ([`request_activation_token`]) is one: the compositor
//! takes the input that asked for it as its evidence that the user wants the
//! new window focused, and a request made from a timer or a worker thread
//! would name no input at all. Such a request asks [`is_active`] and is
//! refused outside an action.
//!
//! A button's release counts as well as its press, as a browser grants user
//! activation on `pointerup`: rinch fires some clicks on the release (an
//! element with a pending drag whose threshold was never crossed clicks
//! when the button comes up), and an `onmouseup` is a deliberate act too.
//!
//! [`RinchApp::handle_event`]: super::RinchApp::handle_event
//! [`request_activation_token`]: crate::activation::request_activation_token

use std::cell::Cell;

use rinch_platform::PlatformEvent;

thread_local! {
    /// How many action dispatches are running on this thread: more than one
    /// when a handler dispatches another event.
    static ACTIONS: Cell<u32> = const { Cell::new(0) };
}

/// Marks an action's dispatch while it lives: see [`enter`].
#[must_use = "the dispatch is a user action only while the guard lives"]
pub(crate) struct UserAction(());

impl Drop for UserAction {
    fn drop(&mut self) {
        ACTIONS.with(|a| a.set(a.get().saturating_sub(1)));
    }
}

/// The guard that makes [`is_active`] true for the dispatch of `event`, when
/// it is a user action: a key going down (an auto-repeat too, which is the
/// key still held), or a pointer button or contact going down or up.
/// Anything else (a key's release, a move, a wheel, a focus change) is not
/// the user asking for something, and gets none.
pub(crate) fn enter(event: &PlatformEvent) -> Option<UserAction> {
    matches!(
        event,
        PlatformEvent::KeyDown { .. }
            | PlatformEvent::MouseDown { .. }
            | PlatformEvent::MouseUp { .. }
    )
    .then(|| {
        ACTIONS.with(|a| a.set(a.get() + 1));
        UserAction(())
    })
}

/// Whether a user action is being dispatched on this thread right now: the
/// code running is a key or pointer handler, or something it called.
#[cfg(any(feature = "desktop", feature = "android", test))]
pub(crate) fn is_active() -> bool {
    ACTIONS.with(|a| a.get() > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rinch_platform::{KeyCode, KeyRepeat, Modifiers, MouseButton};

    fn key(down: bool) -> PlatformEvent {
        if down {
            PlatformEvent::KeyDown {
                key: KeyCode::Enter,
                logical_key: None,
                text: None,
                modifiers: Modifiers::default(),
                repeat: KeyRepeat::Fresh,
            }
        } else {
            PlatformEvent::KeyUp {
                key: KeyCode::Enter,
                logical_key: None,
                modifiers: Modifiers::default(),
            }
        }
    }

    #[test]
    fn a_key_press_and_a_button_press_or_release_are_actions_while_their_guards_live() {
        assert!(!is_active());
        let key = enter(&key(true));
        assert!(key.is_some() && is_active());
        let (x, y, button) = (1.0, 1.0, MouseButton::Left);
        let press = enter(&PlatformEvent::MouseDown { x, y, button });
        let release = enter(&PlatformEvent::MouseUp { x, y, button });
        assert!(press.is_some() && release.is_some());
        drop(key);
        drop(press);
        assert!(is_active(), "the release still runs");
        drop(release);
        assert!(!is_active());
    }

    #[test]
    fn a_key_release_a_move_a_wheel_or_a_focus_change_is_none() {
        for event in [
            key(false),
            PlatformEvent::MouseMove { x: 1.0, y: 1.0 },
            PlatformEvent::MouseWheel {
                x: 1.0,
                y: 1.0,
                delta_x: 0.0,
                delta_y: 10.0,
            },
            PlatformEvent::WindowFocus(true),
        ] {
            assert!(enter(&event).is_none(), "{event:?}");
            assert!(!is_active());
        }
    }
}
