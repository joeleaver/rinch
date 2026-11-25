//! Reading the activity's input queue: touch and hardware key events.

use std::time::Instant;

use android_activity::AndroidApp;
use android_activity::InputStatus;
use android_activity::input::{KeyAction, KeyMapChar, MotionAction};
use rinch_platform::{KeyCode, Modifiers, PlatformEvent};

use crate::shell::android_key::{KeyPhase, KeyTranslator, MapChar, RawKey};
use crate::shell::touch_gesture::{EventClock, TouchAction, TouchGesture};

/// The motion actions the recogniser reacts to. Everything else — pointer index
/// churn from a second finger, button events from a mouse — is `Other`, which it
/// ignores.
fn touch_action(action: MotionAction) -> TouchAction {
    match action {
        MotionAction::Down => TouchAction::Down,
        MotionAction::Move => TouchAction::Move,
        MotionAction::Up => TouchAction::Up,
        MotionAction::Cancel => TouchAction::Cancel,
        MotionAction::HoverMove => TouchAction::HoverMove,
        _ => TouchAction::Other,
    }
}

/// `now` is sampled once per loop iteration by the caller and threaded through,
/// so every event in one drain — and the timers that run after it — agree on
/// what time it is.
pub(super) fn collect_input_events(
    android_app: &AndroidApp,
    gesture: &mut TouchGesture,
    event_clock: &mut EventClock,
    scale_factor: f64,
    now: Instant,
    keys: &mut KeyTranslator,
) -> Vec<PlatformEvent> {
    let mut events = Vec::new();

    match android_app.input_events_iter() {
        Ok(mut iter) => {
            while iter.next(|input_event| {
                use android_activity::input::InputEvent;
                match input_event {
                    InputEvent::MotionEvent(motion) => {
                        let ptr = motion.pointer_at_index(0);
                        let x = (ptr.x() as f64 / scale_factor) as f32;
                        let y = (ptr.y() as f64 / scale_factor) as f32;
                        // **The event's own timestamp, not `now`.** Every event
                        // in one drain used to be stamped with the instant the
                        // loop woke, which quantised the finger's motion to the
                        // frame grid: correct positions, wrong times. A fling's
                        // launch speed is a distance divided by a duration, and
                        // a duration rounded to the nearest frame is wrong by up
                        // to 8.3ms of the roughly 58ms the speed is measured
                        // over. Card K40 measured the difference as 26% of
                        // variation against 2%. `EventClock` is what makes
                        // reading the device's own clock safe without asserting
                        // that it counts in the same base we do.
                        let sampled_at = event_clock.instant_for(now, motion.event_time());
                        gesture.process(
                            touch_action(motion.action()),
                            x,
                            y,
                            sampled_at,
                            &mut events,
                        );
                    }
                    InputEvent::KeyEvent(key) => {
                        let meta = key.meta_state();
                        let modifiers = Modifiers {
                            ctrl: meta.ctrl_on(),
                            shift: meta.shift_on(),
                            alt: meta.alt_on(),
                            meta: false,
                        };

                        // Only what needs the device is done here: read the
                        // event and ask the key character map. What the event
                        // becomes is `android_key`'s, which host tests compile
                        // (issue #479).
                        let phase = match key.action() {
                            KeyAction::Down => Some(KeyPhase::Down {
                                repeat_count: key.repeat_count(),
                            }),
                            KeyAction::Up => Some(KeyPhase::Up),
                            _ => None,
                        };
                        if let Some(phase) = phase {
                            let map = android_app.device_key_character_map(key.device_id()).ok();
                            let ch = match map
                                .as_ref()
                                .map(|map| map.get(key.key_code(), key.meta_state()))
                            {
                                Some(Ok(KeyMapChar::Unicode(ch))) => MapChar::Unicode(ch),
                                Some(Ok(KeyMapChar::CombiningAccent(accent))) => {
                                    MapChar::CombiningAccent(accent)
                                }
                                _ => MapChar::None,
                            };
                            let raw = RawKey {
                                id: (key.device_id(), key.key_code().into()),
                                phase,
                                key: map_android_keycode(key.key_code()),
                                ch,
                                modifiers,
                            };
                            let event = keys.translate(raw, |accent, ch| {
                                map.as_ref()
                                    .and_then(|map| map.get_dead_char(accent, ch).ok().flatten())
                            });
                            events.extend(event);
                        }
                    }
                    _ => {}
                }
                InputStatus::Handled
            }) {}
        }
        Err(e) => {
            log::warn!("input_events_iter failed: {e:?}");
        }
    }

    // A press that has been held still long enough is a context menu. This sits
    // beside the momentum tick because both are clocks the finger is not
    // driving, and a turn of the event loop is what turns them. Both are handed
    // the same `now`, and since card K39 the fling needs it: the loop's rate is
    // the panel's rate (K37) and is therefore not a constant any curve may be
    // written in terms of.
    gesture.tick_long_press(now, &mut events);

    // Apply momentum scrolling
    gesture.tick_momentum(now, &mut events);

    events
}

fn map_android_keycode(keycode: android_activity::input::Keycode) -> Option<KeyCode> {
    use android_activity::input::Keycode as AK;
    match keycode {
        AK::DpadUp => Some(KeyCode::ArrowUp),
        AK::DpadDown => Some(KeyCode::ArrowDown),
        AK::DpadLeft => Some(KeyCode::ArrowLeft),
        AK::DpadRight => Some(KeyCode::ArrowRight),
        AK::MoveHome => Some(KeyCode::Home),
        AK::MoveEnd => Some(KeyCode::End),
        AK::PageUp => Some(KeyCode::PageUp),
        AK::PageDown => Some(KeyCode::PageDown),
        AK::Enter | AK::NumpadEnter => Some(KeyCode::Enter),
        AK::Tab => Some(KeyCode::Tab),
        AK::Escape | AK::Back => Some(KeyCode::Escape),
        AK::Del => Some(KeyCode::Backspace),
        AK::ForwardDel => Some(KeyCode::Delete),
        AK::Space => Some(KeyCode::Space),
        AK::A => Some(KeyCode::KeyA),
        AK::B => Some(KeyCode::KeyB),
        AK::C => Some(KeyCode::KeyC),
        AK::D => Some(KeyCode::KeyD),
        AK::E => Some(KeyCode::KeyE),
        AK::F => Some(KeyCode::KeyF),
        AK::G => Some(KeyCode::KeyG),
        AK::H => Some(KeyCode::KeyH),
        AK::I => Some(KeyCode::KeyI),
        AK::J => Some(KeyCode::KeyJ),
        AK::K => Some(KeyCode::KeyK),
        AK::L => Some(KeyCode::KeyL),
        AK::M => Some(KeyCode::KeyM),
        AK::N => Some(KeyCode::KeyN),
        AK::O => Some(KeyCode::KeyO),
        AK::P => Some(KeyCode::KeyP),
        AK::Q => Some(KeyCode::KeyQ),
        AK::R => Some(KeyCode::KeyR),
        AK::S => Some(KeyCode::KeyS),
        AK::T => Some(KeyCode::KeyT),
        AK::U => Some(KeyCode::KeyU),
        AK::V => Some(KeyCode::KeyV),
        AK::W => Some(KeyCode::KeyW),
        AK::X => Some(KeyCode::KeyX),
        AK::Y => Some(KeyCode::KeyY),
        AK::Z => Some(KeyCode::KeyZ),
        AK::Keycode0 => Some(KeyCode::Digit0),
        AK::Keycode1 => Some(KeyCode::Digit1),
        AK::Keycode2 => Some(KeyCode::Digit2),
        AK::Keycode3 => Some(KeyCode::Digit3),
        AK::Keycode4 => Some(KeyCode::Digit4),
        AK::Keycode5 => Some(KeyCode::Digit5),
        AK::Keycode6 => Some(KeyCode::Digit6),
        AK::Keycode7 => Some(KeyCode::Digit7),
        AK::Keycode8 => Some(KeyCode::Digit8),
        AK::Keycode9 => Some(KeyCode::Digit9),
        AK::ShiftLeft => Some(KeyCode::ShiftLeft),
        AK::ShiftRight => Some(KeyCode::ShiftRight),
        AK::CtrlLeft => Some(KeyCode::ControlLeft),
        AK::CtrlRight => Some(KeyCode::ControlRight),
        AK::AltLeft => Some(KeyCode::AltLeft),
        AK::AltRight => Some(KeyCode::AltRight),
        _ => None,
    }
}
