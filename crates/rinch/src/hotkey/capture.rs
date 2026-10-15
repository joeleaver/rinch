//! [`HotkeyCapture`]: turn the keys a user presses in a settings field into a
//! shortcut string ("press the keys you want").
//!
//! The capture reads the app's **own** key events — a focused field's, not the
//! whole system's — so it needs no permission and grabs nothing. Feed it every
//! key event the field receives ([`HotkeyCapture::feed`], from an
//! `on_key` of `register_focus_target` or a keyboard interceptor installed while
//! capturing) and stop at [`CaptureStep::Done`].
//!
//! The rules:
//! - A non-modifier key completes a **chord**: the modifiers held when it went
//!   down, unsided (`Ctrl`, not `LeftCtrl`), plus the key — `"Ctrl+Alt+Space"`.
//!   Unsided because a plain chord is what the OS hot-key APIs reserve and
//!   report conflicts for.
//! - Modifiers pressed and released with no other key are a **modifier-only**
//!   binding, sided — `"RightCtrl"`, or `"RightCtrl+RightShift"` with the last
//!   one pressed as its trigger. Sided because "hold Right Ctrl" is the point
//!   of such a binding.
//! - `Escape` alone cancels.

use std::str::FromStr;

use rinch_core::{KeyEventData, KeyEventKind};
use winit::keyboard::KeyCode;

use super::shortcut::{Modifier, Shortcut};

/// What a key event did to a capture.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CaptureStep {
    /// Still capturing. The string is what is held so far, for a live preview
    /// (`"Ctrl+Alt+"`), empty when nothing is.
    Pending(String),
    /// The user finished a shortcut.
    Done(Shortcut),
    /// The user pressed Escape alone.
    Cancelled,
}

/// A "press the keys you want" capture. See the module docs.
#[derive(Default, Debug, Clone)]
pub struct HotkeyCapture {
    down: Vec<KeyCode>,
    /// The modifiers pressed since the capture last had nothing held, in
    /// press order, while no non-modifier key went down.
    mods_pressed: Vec<KeyCode>,
}

impl HotkeyCapture {
    /// A fresh capture.
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed a key event of the app's own (its `code` is read, not its `key`).
    /// An event with a `code` that is not a W3C code name is ignored.
    pub fn feed(&mut self, ev: &KeyEventData) -> CaptureStep {
        match KeyCode::from_str(&ev.code) {
            Ok(code) => self.feed_code(code, ev.kind == KeyEventKind::Down),
            Err(_) => self.pending(),
        }
    }

    /// Feed one physical key going down (`down = true`) or up.
    pub fn feed_code(&mut self, code: KeyCode, down: bool) -> CaptureStep {
        let is_mod = Modifier::of_code(code).is_some();
        if down {
            if self.down.contains(&code) {
                return self.pending();
            }
            if !is_mod {
                let held: Vec<KeyCode> = self.down.clone();
                let only_mods_held = held.iter().all(|k| Modifier::of_code(*k).is_some());
                if code == KeyCode::Escape && held.is_empty() {
                    *self = Self::default();
                    return CaptureStep::Cancelled;
                }
                if only_mods_held {
                    let mut s = String::new();
                    for m in [
                        Modifier::Ctrl,
                        Modifier::Alt,
                        Modifier::Shift,
                        Modifier::Super,
                    ] {
                        if held
                            .iter()
                            .any(|k| Modifier::of_code(*k).map(|(hm, _)| hm) == Some(m))
                        {
                            s.push_str(&format!("{}+", m.name()));
                        }
                    }
                    s.push_str(&super::shortcut::key_display(code));
                    *self = Self::default();
                    if let Ok(sc) = Shortcut::parse(&s) {
                        return CaptureStep::Done(sc);
                    }
                    return CaptureStep::Pending(String::new());
                }
            }
            self.down.push(code);
            if is_mod && self.down.iter().all(|k| Modifier::of_code(*k).is_some()) {
                self.mods_pressed.push(code);
            } else {
                self.mods_pressed.clear();
            }
            self.pending()
        } else {
            self.down.retain(|k| *k != code);
            if is_mod && !self.mods_pressed.is_empty() && self.down.is_empty() {
                // Modifier-only: everything pressed went up with no other key.
                let (last, init) = self.mods_pressed.split_last().expect("non-empty");
                let mut s = String::new();
                for k in init {
                    if *k != *last {
                        s.push_str(&format!("{k}+"));
                    }
                }
                s.push_str(&format!("{last}"));
                *self = Self::default();
                if let Ok(sc) = Shortcut::parse(&s) {
                    return CaptureStep::Done(sc);
                }
                return CaptureStep::Pending(String::new());
            }
            if self.down.is_empty() {
                self.mods_pressed.clear();
            }
            self.pending()
        }
    }

    fn pending(&self) -> CaptureStep {
        let mut s = String::new();
        for m in [
            Modifier::Ctrl,
            Modifier::Alt,
            Modifier::Shift,
            Modifier::Super,
        ] {
            if self
                .down
                .iter()
                .any(|k| Modifier::of_code(*k).map(|(hm, _)| hm) == Some(m))
            {
                s.push_str(&format!("{}+", m.name()));
            }
        }
        CaptureStep::Pending(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use KeyCode::*;

    fn feed(keys: &[(KeyCode, bool)]) -> Vec<CaptureStep> {
        let mut c = HotkeyCapture::new();
        keys.iter().map(|&(k, d)| c.feed_code(k, d)).collect()
    }

    fn done(steps: &[CaptureStep]) -> Option<String> {
        steps.iter().find_map(|s| match s {
            CaptureStep::Done(sc) => Some(sc.to_string()),
            _ => None,
        })
    }

    #[test]
    fn a_chord_is_captured_unsided_on_its_key() {
        let steps = feed(&[(ControlRight, true), (AltLeft, true), (Space, true)]);
        assert_eq!(steps[1], CaptureStep::Pending("Ctrl+Alt+".into()));
        assert_eq!(done(&steps).as_deref(), Some("Ctrl+Alt+Space"));
    }

    #[test]
    fn a_lone_modifier_is_captured_sided() {
        let steps = feed(&[
            (ControlRight, true),
            (ControlRight, true),
            (ControlRight, false),
        ]);
        assert_eq!(done(&steps).as_deref(), Some("RightCtrl"));
        let steps = feed(&[
            (ControlLeft, true),
            (ShiftRight, true),
            (ShiftRight, false),
            (ControlLeft, false),
        ]);
        assert_eq!(done(&steps).as_deref(), Some("LeftCtrl+RightShift"));
    }

    #[test]
    fn escape_alone_cancels_and_escape_with_a_modifier_is_a_chord() {
        assert_eq!(feed(&[(Escape, true)]), vec![CaptureStep::Cancelled]);
        assert_eq!(
            done(&feed(&[(ShiftLeft, true), (Escape, true)])).as_deref(),
            Some("Shift+Escape")
        );
    }

    #[test]
    fn a_modifier_released_after_a_key_is_not_modifier_only() {
        // Ctrl held, a stray non-modifier released earlier: the key completes
        // the chord the moment it goes down, so nothing else is captured.
        let steps = feed(&[
            (ControlLeft, true),
            (KeyK, true),
            (KeyK, false),
            (ControlLeft, false),
        ]);
        assert_eq!(done(&steps).as_deref(), Some("Ctrl+K"));
        assert_eq!(
            steps
                .iter()
                .filter(|s| matches!(s, CaptureStep::Done(_)))
                .count(),
            1
        );
    }

    #[test]
    fn every_capture_parses_back() {
        for s in [
            done(&feed(&[(MetaLeft, true), (F13, true)])),
            done(&feed(&[(AltRight, true), (AltRight, false)])),
        ] {
            let s = s.expect("captured");
            assert_eq!(Shortcut::parse(&s).unwrap().to_string(), s);
        }
    }
}
