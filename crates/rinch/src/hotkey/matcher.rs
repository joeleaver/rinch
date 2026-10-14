//! The press/release state machine every key-stream backend shares (the
//! Windows low-level hook, the macOS event tap, X11's XInput2 raw events).
//!
//! It is fed every physical key going down or up, system-wide, and answers
//! which bindings that pressed or released. Pure: no OS, no threads, so the
//! rules are tested here once rather than three times.
//!
//! The rules:
//! - A binding is **pressed** by the key that completes it while exactly its
//!   modifiers are held ([`Shortcut::completes`]). A held key going down again
//!   is auto-repeat and presses nothing.
//! - It is **released** by letting go of its trigger key — or, for a
//!   modifier-only binding, of any of its modifiers ([`Shortcut::released_by`]).
//!   Releasing a modifier of a chord while the chord's key is still held keeps
//!   it pressed, as the OS hot-key APIs do.
//! - A pressed binding always gets its release: [`Matcher::release_all`] is the
//!   clearing condition for a stream that loses key-ups (a hook that stops
//!   seeing input while an elevated window has focus, a tap the OS disabled),
//!   and [`Matcher::forget_key`] lets a backend that can ask the OS whether a
//!   key is still down drop one it missed.

use winit::keyboard::KeyCode;

use super::HotkeyState;
use super::shortcut::Shortcut;

/// A binding's key in the main-thread registry.
pub(crate) type BindingKey = u64;

#[derive(Default)]
pub(crate) struct Matcher {
    bindings: Vec<(BindingKey, Shortcut, bool)>,
    down: Vec<KeyCode>,
}

impl Matcher {
    pub(crate) fn add(&mut self, key: BindingKey, shortcut: Shortcut) {
        self.bindings.retain(|b| b.0 != key);
        self.bindings.push((key, shortcut, false));
    }

    /// Remove a binding. A binding removed while pressed gets no release: its
    /// owner unregistered it and is not listening.
    pub(crate) fn remove(&mut self, key: BindingKey) {
        self.bindings.retain(|b| b.0 != key);
    }

    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.bindings.is_empty()
    }

    /// Whether any binding is pressed now (a backend polls the keys while one is).
    #[cfg_attr(not(any(windows, test)), allow(dead_code))]
    pub(crate) fn any_pressed(&self) -> bool {
        self.bindings.iter().any(|b| b.2)
    }

    /// The keys the matcher believes are down.
    #[cfg_attr(not(any(windows, target_os = "macos", test)), allow(dead_code))]
    pub(crate) fn down_keys(&self) -> &[KeyCode] {
        &self.down
    }

    /// Feed one key transition; push what it pressed or released onto `out`.
    pub(crate) fn key(
        &mut self,
        code: KeyCode,
        down: bool,
        out: &mut Vec<(BindingKey, HotkeyState)>,
    ) {
        if down {
            if self.down.contains(&code) {
                return; // auto-repeat
            }
            let held = self.down.clone();
            self.down.push(code);
            for (key, shortcut, pressed) in &mut self.bindings {
                if !*pressed && shortcut.completes(code, &held) {
                    *pressed = true;
                    out.push((*key, HotkeyState::Pressed));
                }
            }
        } else {
            self.down.retain(|&k| k != code);
            for (key, shortcut, pressed) in &mut self.bindings {
                if *pressed && shortcut.released_by(code) {
                    *pressed = false;
                    out.push((*key, HotkeyState::Released));
                }
            }
        }
    }

    /// The stream says `code` went up without our seeing it: treat it as a
    /// key-up now.
    #[cfg_attr(not(any(windows, test)), allow(dead_code))]
    pub(crate) fn forget_key(&mut self, code: KeyCode, out: &mut Vec<(BindingKey, HotkeyState)>) {
        if self.down.contains(&code) {
            self.key(code, false, out);
        }
    }

    /// Release every pressed binding and forget every key: the stream lost
    /// track (it was disabled, the session was locked).
    pub(crate) fn release_all(&mut self, out: &mut Vec<(BindingKey, HotkeyState)>) {
        self.down.clear();
        for (key, _, pressed) in &mut self.bindings {
            if *pressed {
                *pressed = false;
                out.push((*key, HotkeyState::Released));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use HotkeyState::*;
    use KeyCode::*;

    fn m(bindings: &[(u64, &str)]) -> Matcher {
        let mut m = Matcher::default();
        for (k, s) in bindings {
            m.add(*k, Shortcut::parse(s).unwrap());
        }
        m
    }

    fn run(m: &mut Matcher, keys: &[(KeyCode, bool)]) -> Vec<(u64, HotkeyState)> {
        let mut out = Vec::new();
        for &(k, d) in keys {
            m.key(k, d, &mut out);
        }
        out
    }

    #[test]
    fn a_chord_presses_once_and_releases_on_its_key() {
        let mut mm = m(&[(1, "Ctrl+Alt+Space")]);
        let out = run(
            &mut mm,
            &[
                (ControlLeft, true),
                (AltLeft, true),
                (Space, true),
                (Space, true),
                (Space, true),
                (AltLeft, false),
                (Space, false),
                (ControlLeft, false),
            ],
        );
        assert_eq!(out, vec![(1, Pressed), (1, Released)]);
    }

    /// Auto-repeat is not a press: F5 went down with Ctrl held, so letting go
    /// of Ctrl and holding on does not make its repeats a press of `F5`.
    #[test]
    fn a_repeat_after_the_modifiers_changed_presses_nothing() {
        let mut mm = m(&[(3, "F5")]);
        let out = run(
            &mut mm,
            &[
                (ControlLeft, true),
                (F5, true),
                (ControlLeft, false),
                (F5, true),
                (F5, true),
                (F5, false),
            ],
        );
        assert!(out.is_empty(), "{out:?}");
        assert_eq!(
            run(&mut mm, &[(F5, true), (F5, false)]),
            vec![(3, Pressed), (3, Released)]
        );
    }

    #[test]
    fn a_modifier_only_binding_is_hold_to_talk() {
        let mut mm = m(&[(7, "RightCtrl")]);
        assert_eq!(
            run(&mut mm, &[(ControlRight, true), (ControlRight, true)]),
            vec![(7, Pressed)]
        );
        assert_eq!(run(&mut mm, &[(ControlRight, false)]), vec![(7, Released)]);
        // Left Ctrl, and Right Ctrl as part of a chord, press nothing.
        assert!(run(&mut mm, &[(ControlLeft, true), (ControlLeft, false)]).is_empty());
        assert!(
            run(
                &mut mm,
                &[
                    (KeyC, true),
                    (ControlRight, true),
                    (ControlRight, false),
                    (KeyC, false)
                ]
            )
            .is_empty()
        );
    }

    #[test]
    fn a_two_modifier_binding_releases_on_either() {
        let mut mm = m(&[(2, "Ctrl+Shift")]);
        assert_eq!(
            run(
                &mut mm,
                &[
                    (ControlLeft, true),
                    (ShiftRight, true),
                    (ControlLeft, false),
                    (ShiftRight, false)
                ]
            ),
            vec![(2, Pressed), (2, Released)]
        );
    }

    #[test]
    fn bindings_sharing_keys_fire_independently() {
        let mut mm = m(&[(1, "Ctrl+Space"), (2, "Ctrl+Shift+Space")]);
        let out = run(
            &mut mm,
            &[
                (ControlLeft, true),
                (ShiftLeft, true),
                (Space, true),
                (Space, false),
            ],
        );
        assert_eq!(out, vec![(2, Pressed), (2, Released)]);
    }

    #[test]
    fn a_lost_key_up_is_cleared_by_release_all_and_forget_key() {
        let mut mm = m(&[(1, "RightCtrl"), (2, "Ctrl+K")]);
        run(&mut mm, &[(ControlRight, true)]);
        assert!(mm.any_pressed());
        let mut out = Vec::new();
        mm.forget_key(ControlRight, &mut out);
        assert_eq!(out, vec![(1, Released)]);
        assert!(!mm.any_pressed() && mm.down_keys().is_empty());
        // Without the clear, a stranded key would block the next press of a
        // *different* binding's exact-modifier check; release_all resets it.
        run(&mut mm, &[(ControlLeft, true), (KeyK, true)]);
        let mut out = Vec::new();
        mm.release_all(&mut out);
        assert_eq!(out, vec![(2, Released)]);
        assert_eq!(
            run(&mut mm, &[(ControlLeft, true), (KeyK, true)]),
            vec![(2, Pressed)]
        );
    }

    #[test]
    fn a_removed_binding_gets_no_release() {
        let mut mm = m(&[(1, "RightCtrl")]);
        run(&mut mm, &[(ControlRight, true)]);
        mm.remove(1);
        assert!(run(&mut mm, &[(ControlRight, false)]).is_empty());
        assert!(mm.is_empty());
    }
}
