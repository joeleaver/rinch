//! What the Android shell answers the system for each key event (#3062).
//!
//! android-activity asks the input callback for an `InputStatus` per event.
//! `Handled` means the app used the key and the system's default for it never
//! runs; `Unhandled` hands it back, and the system does what it does for an
//! app that ignores the key: the volume keys change the volume, the media keys
//! reach the media session, and Back goes back (the Activity finishes, or on
//! Android 12 and later a root Activity moves to the background). The shell
//! used to answer `Handled` for everything, so none of that ever happened in
//! a rinch app.
//!
//! The rules, over plain data so host tests compile them (`android_runtime`
//! builds for `target_os = "android"` only):
//!
//! - **A key rinch does not map is handed back.** The translator
//!   (`android_key`) drops an event whose key code has no
//!   [`KeyCode`](rinch_platform::KeyCode) and that types no character, and the
//!   runtime drops an action rinch does not read (`ACTION_MULTIPLE`). The app
//!   never saw those, so they are not the app's. A dead key is the exception
//!   ([`undelivered`]): the translator keeps its accent for the next
//!   character, so it was used.
//! - **Back is handed back when nothing used it.** Back reaches the app as
//!   Escape, so a popup, a modal, a dismiss entry or an app key handler can
//!   take it first, as Chrome on Android closes the top popup before it goes
//!   back. The press is answered from
//!   [`RinchApp::key_down_unused`](crate::app::RinchApp::key_down_unused),
//!   read right after the press was dispatched inside the input callback.
//! - **A Back release follows its press.** The system goes back on the
//!   release, and only for a press it saw unhandled (Android's
//!   `KeyEvent.startTracking`). So the release is handed back exactly when its
//!   press was, whatever the app did with the release itself; a release whose
//!   press was not handed back (or was never seen) is the app's.
//! - **Every other key the app received stays the app's**, used or not, as
//!   before. Escape on a hardware keyboard is one of them: Android's generic
//!   key map would turn an unhandled Escape into a Back, and an app that does
//!   nothing with Escape should not quit on it.
//!
//! Not checked on a device.

use std::collections::HashSet;

use crate::shell::android_key::{KeyPhase, MapChar};

/// The answer for one key event; the runtime maps it to android-activity's
/// `InputStatus`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KeyStatus {
    /// The app used the key: the system's default does not run.
    Handled,
    /// Handed back to the system, whose default runs.
    Unhandled,
}

/// What happened to one key event inside the input callback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KeyDelivery {
    /// The app never saw it: no rinch key and no character, or an action rinch
    /// does not read.
    NotDelivered,
    /// Dispatched to the app as a `KeyDown` (a press or an auto-repeat);
    /// `unused` is [`RinchApp::key_down_unused`](crate::app::RinchApp::key_down_unused)
    /// right after.
    Pressed { unused: bool },
    /// Dispatched to the app as a `KeyUp`.
    Released,
}

/// The state the answers need between events: which Back keys were handed
/// back on their press, by `RawKey::id` (device, key code).
#[derive(Debug, Default)]
pub(crate) struct KeyAnswers {
    handed_back: HashSet<(i32, u32)>,
}

impl KeyAnswers {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// The answer for one key event. `id` is the key's `(device id, Android
    /// key code)`, `phase` the half of the keystroke it is (for an event that
    /// was delivered, the one the app got), and `is_back` whether it is
    /// `KEYCODE_BACK`.
    pub(crate) fn answer(
        &mut self,
        id: (i32, u32),
        is_back: bool,
        phase: Option<KeyPhase>,
        delivery: KeyDelivery,
    ) -> KeyStatus {
        match delivery {
            KeyDelivery::NotDelivered => {
                // An undelivered Back release (its press was never seen)
                // still owes nothing; drop any record so it cannot leak.
                if is_back && phase == Some(KeyPhase::Up) {
                    self.handed_back.remove(&id);
                }
                KeyStatus::Unhandled
            }
            KeyDelivery::Pressed { unused } if is_back => {
                // Each press decides for the release that follows it: an
                // auto-repeat an app took after the press went unused keeps
                // the key, and the other way round.
                if unused {
                    self.handed_back.insert(id);
                    KeyStatus::Unhandled
                } else {
                    self.handed_back.remove(&id);
                    KeyStatus::Handled
                }
            }
            KeyDelivery::Released if is_back => {
                if self.handed_back.remove(&id) {
                    KeyStatus::Unhandled
                } else {
                    KeyStatus::Handled
                }
            }
            KeyDelivery::Pressed { .. } | KeyDelivery::Released => KeyStatus::Handled,
        }
    }
}

/// The delivery of a key event the translator turned into no event. A dead key
/// (a combining accent, such as Option+E on a Mac layout) is one: the
/// translator holds the accent for the next character, so it was used even
/// though the app saw nothing, and both halves are the app's. Anything else it
/// dropped never reached the app.
pub(crate) fn undelivered(ch: MapChar, phase: KeyPhase) -> KeyDelivery {
    match (ch, phase) {
        (MapChar::CombiningAccent(_), KeyPhase::Down { .. }) => {
            KeyDelivery::Pressed { unused: false }
        }
        (MapChar::CombiningAccent(_), KeyPhase::Up) => KeyDelivery::Released,
        _ => KeyDelivery::NotDelivered,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BACK: (i32, u32) = (3, 4);
    const ESCAPE: (i32, u32) = (3, 111);
    const VOLUME_UP: (i32, u32) = (3, 24);
    const DOWN: Option<KeyPhase> = Some(KeyPhase::Down { repeat_count: 0 });
    const REPEAT: Option<KeyPhase> = Some(KeyPhase::Down { repeat_count: 1 });
    const UP: Option<KeyPhase> = Some(KeyPhase::Up);

    fn pressed(unused: bool) -> KeyDelivery {
        KeyDelivery::Pressed { unused }
    }

    #[test]
    fn a_key_rinch_does_not_map_is_handed_back_both_ways() {
        let mut a = KeyAnswers::new();
        for phase in [DOWN, UP, None] {
            assert_eq!(
                a.answer(VOLUME_UP, false, phase, KeyDelivery::NotDelivered),
                KeyStatus::Unhandled,
                "{phase:?}"
            );
        }
    }

    #[test]
    fn an_unused_back_goes_back_press_and_release() {
        let mut a = KeyAnswers::new();
        assert_eq!(
            a.answer(BACK, true, DOWN, pressed(true)),
            KeyStatus::Unhandled
        );
        assert_eq!(
            a.answer(BACK, true, UP, KeyDelivery::Released),
            KeyStatus::Unhandled
        );
    }

    #[test]
    fn a_back_something_took_stays_the_apps_press_and_release() {
        let mut a = KeyAnswers::new();
        assert_eq!(
            a.answer(BACK, true, DOWN, pressed(false)),
            KeyStatus::Handled
        );
        assert_eq!(
            a.answer(BACK, true, UP, KeyDelivery::Released),
            KeyStatus::Handled
        );
    }

    /// The record is per keystroke: after a Back that went back, the next one
    /// a popup takes is kept, release included, and the other way round.
    #[test]
    fn each_back_keystroke_is_answered_on_its_own() {
        let mut a = KeyAnswers::new();
        a.answer(BACK, true, DOWN, pressed(true));
        a.answer(BACK, true, UP, KeyDelivery::Released);
        assert_eq!(
            a.answer(BACK, true, DOWN, pressed(false)),
            KeyStatus::Handled
        );
        assert_eq!(
            a.answer(BACK, true, UP, KeyDelivery::Released),
            KeyStatus::Handled
        );
        a.answer(BACK, true, DOWN, pressed(true));
        assert_eq!(
            a.answer(BACK, true, UP, KeyDelivery::Released),
            KeyStatus::Unhandled
        );
    }

    /// The release follows the last press the app got, auto-repeats included.
    #[test]
    fn a_back_release_follows_its_last_press() {
        let mut a = KeyAnswers::new();
        a.answer(BACK, true, DOWN, pressed(true));
        assert_eq!(
            a.answer(BACK, true, REPEAT, pressed(false)),
            KeyStatus::Handled
        );
        assert_eq!(
            a.answer(BACK, true, UP, KeyDelivery::Released),
            KeyStatus::Handled
        );
    }

    /// A Back already held when the app came to the front: its release alone
    /// is not a back gesture the system tracked, so it stays the app's.
    #[test]
    fn a_back_release_without_its_press_stays_the_apps() {
        let mut a = KeyAnswers::new();
        assert_eq!(
            a.answer(BACK, true, UP, KeyDelivery::Released),
            KeyStatus::Handled
        );
    }

    /// Two devices' Back keys are two keys.
    #[test]
    fn back_keys_on_two_devices_are_answered_apart() {
        let mut a = KeyAnswers::new();
        let other = (9, 4);
        a.answer(BACK, true, DOWN, pressed(true));
        a.answer(other, true, DOWN, pressed(false));
        assert_eq!(
            a.answer(other, true, UP, KeyDelivery::Released),
            KeyStatus::Handled
        );
        assert_eq!(
            a.answer(BACK, true, UP, KeyDelivery::Released),
            KeyStatus::Unhandled
        );
    }

    /// Every other key the app got stays its own, used or not: an unused
    /// Escape is not handed back (Android would turn it into a Back).
    #[test]
    fn any_other_delivered_key_stays_the_apps() {
        let mut a = KeyAnswers::new();
        for unused in [true, false] {
            assert_eq!(
                a.answer(ESCAPE, false, DOWN, pressed(unused)),
                KeyStatus::Handled
            );
            assert_eq!(
                a.answer(ESCAPE, false, UP, KeyDelivery::Released),
                KeyStatus::Handled
            );
        }
    }

    /// An undelivered Back release drops a press record, so a later release
    /// of the same key cannot inherit it.
    #[test]
    fn an_undelivered_back_release_clears_its_record() {
        let mut a = KeyAnswers::new();
        a.answer(BACK, true, DOWN, pressed(true));
        a.answer(BACK, true, UP, KeyDelivery::NotDelivered);
        assert_eq!(
            a.answer(BACK, true, UP, KeyDelivery::Released),
            KeyStatus::Handled
        );
    }

    /// A dead key was used by the translator: both halves stay the app's. A
    /// key with no character and no rinch key code was not.
    #[test]
    fn a_dead_key_is_the_apps_and_an_unmapped_key_is_not() {
        let mut a = KeyAnswers::new();
        let down = KeyPhase::Down { repeat_count: 0 };
        let accent = MapChar::CombiningAccent('\u{301}');
        for (phase, delivery) in [
            (down, KeyDelivery::Pressed { unused: false }),
            (KeyPhase::Up, KeyDelivery::Released),
        ] {
            assert_eq!(undelivered(accent, phase), delivery);
            assert_eq!(
                a.answer(VOLUME_UP, false, Some(phase), undelivered(accent, phase)),
                KeyStatus::Handled
            );
            assert_eq!(undelivered(MapChar::None, phase), KeyDelivery::NotDelivered);
            assert_eq!(
                undelivered(MapChar::Unicode('a'), phase),
                KeyDelivery::NotDelivered
            );
        }
    }
}
