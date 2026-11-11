//! Whether the code running now runs because the user pressed something:
//! a key, a pointer button or a touch, while [`RinchApp::handle_event`]
//! dispatches it (#2501).
//!
//! Some requests are honoured only as the answer to a user action. The
//! xdg-activation token ([`request_activation_token`]) is one: the compositor
//! takes the input that asked for it as its evidence that the user wants the
//! new window focused, and a request made from a timer or a worker thread
//! would name no input at all. Such a request asks [`is_active`] and is
//! refused outside a press.
//!
//! [`RinchApp::handle_event`]: super::RinchApp::handle_event
//! [`request_activation_token`]: crate::activation::request_activation_token

use std::cell::Cell;

use rinch_platform::PlatformEvent;

thread_local! {
    /// How many press dispatches are running on this thread: more than one
    /// when a handler dispatches another event (`MouseWheelLines` re-enters
    /// `handle_event`).
    static PRESSES: Cell<u32> = const { Cell::new(0) };
}

/// Marks a press dispatch while it lives: see [`enter`].
#[must_use = "the dispatch is a press only while the guard lives"]
pub(crate) struct Press(());

impl Drop for Press {
    fn drop(&mut self) {
        PRESSES.with(|p| p.set(p.get().saturating_sub(1)));
    }
}

/// The guard that makes [`is_active`] true for the dispatch of `event`, when
/// it is a press: a key going down (an auto-repeat too, which is the key
/// still held) or a pointer button or contact going down. Anything else
/// (a release, a move, a wheel, a focus change) is not the user asking for
/// something, and gets none.
pub(crate) fn enter(event: &PlatformEvent) -> Option<Press> {
    matches!(
        event,
        PlatformEvent::KeyDown { .. } | PlatformEvent::MouseDown { .. }
    )
    .then(|| {
        PRESSES.with(|p| p.set(p.get() + 1));
        Press(())
    })
}

/// Whether a press is being dispatched on this thread right now: the code
/// running is a key or pointer handler, or something it called.
#[cfg(any(feature = "desktop", feature = "android", test))]
pub(crate) fn is_active() -> bool {
    PRESSES.with(|p| p.get() > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rinch_platform::{KeyCode, KeyRepeat, Modifiers, MouseButton};

    fn key_down() -> PlatformEvent {
        PlatformEvent::KeyDown {
            key: KeyCode::Enter,
            logical_key: None,
            text: None,
            modifiers: Modifiers::default(),
            repeat: KeyRepeat::Fresh,
        }
    }

    #[test]
    fn a_press_is_active_while_its_guard_lives() {
        assert!(!is_active());
        let key = enter(&key_down());
        assert!(key.is_some() && is_active());
        let button = PlatformEvent::MouseDown {
            x: 1.0,
            y: 1.0,
            button: MouseButton::Left,
        };
        let nested = enter(&button);
        drop(key);
        assert!(is_active(), "the nested press still runs");
        drop(nested);
        assert!(!is_active());
    }

    #[test]
    fn a_release_a_move_or_a_focus_change_is_no_press() {
        for event in [
            PlatformEvent::MouseUp {
                x: 1.0,
                y: 1.0,
                button: MouseButton::Left,
            },
            PlatformEvent::MouseMove { x: 1.0, y: 1.0 },
            PlatformEvent::WindowFocus(true),
        ] {
            assert!(enter(&event).is_none(), "{event:?}");
            assert!(!is_active());
        }
    }
}
