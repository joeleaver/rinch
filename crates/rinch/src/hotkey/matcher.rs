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
//!   and [`Matcher::forget_key`] / [`Matcher::key_reconciled`] let a backend
//!   that can ask the OS whether a key is still down drop one it missed.
//! - Lock keys (Caps/Num/Scroll Lock) are never "another key held".
//! - [`Matcher::chorded`] says whether another key went down while a binding
//!   was pressed.

use winit::keyboard::KeyCode;

use super::HotkeyState;
use super::shortcut::Shortcut;

/// A binding's key in the main-thread registry.
pub(crate) type BindingKey = u64;

struct Bound {
    key: BindingKey,
    shortcut: Shortcut,
    pressed: bool,
    /// Another key went down while this binding was pressed (see
    /// [`Matcher::chorded`]).
    chorded: bool,
}

#[derive(Default)]
pub(crate) struct Matcher {
    bindings: Vec<Bound>,
    down: Vec<KeyCode>,
}

/// Caps Lock, Num Lock and Scroll Lock: a lock key's state is not a key
/// being held. macOS reports Caps Lock as one `flagsChanged` per state change,
/// so a stream can leave it "down" for as long as the lock is on.
pub(crate) fn is_lock_key(code: KeyCode) -> bool {
    matches!(
        code,
        KeyCode::CapsLock | KeyCode::NumLock | KeyCode::ScrollLock
    )
}

impl Matcher {
    pub(crate) fn add(&mut self, key: BindingKey, shortcut: Shortcut) {
        self.bindings.retain(|b| b.key != key);
        self.bindings.push(Bound {
            key,
            shortcut,
            pressed: false,
            chorded: false,
        });
    }

    /// Remove a binding. A binding removed while pressed gets no release: its
    /// owner unregistered it and is not listening.
    pub(crate) fn remove(&mut self, key: BindingKey) {
        self.bindings.retain(|b| b.key != key);
    }

    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.bindings.is_empty()
    }

    /// Whether any binding is pressed now.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn any_pressed(&self) -> bool {
        self.bindings.iter().any(|b| b.pressed)
    }

    /// Whether another key went down while `key` was last pressed — read with
    /// its `Released`. A modifier-only binding (`RightCtrl`) fires on the
    /// modifier alone, before anyone can know a chord (`Right Ctrl+C`) will
    /// follow; this says, at the release, that one did.
    pub(crate) fn chorded(&self, key: BindingKey) -> bool {
        self.bindings.iter().any(|b| b.key == key && b.chorded)
    }

    /// The keys the matcher believes are down.
    #[cfg_attr(not(any(windows, test)), allow(dead_code))]
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
            for b in &mut self.bindings {
                if b.pressed {
                    if !is_lock_key(code) {
                        b.chorded = true;
                    }
                } else if b.shortcut.completes(code, &held) {
                    b.pressed = true;
                    b.chorded = false;
                    out.push((b.key, HotkeyState::Pressed));
                }
            }
        } else {
            self.down.retain(|&k| k != code);
            for b in &mut self.bindings {
                if b.pressed && b.shortcut.released_by(code) {
                    b.pressed = false;
                    out.push((b.key, HotkeyState::Released));
                }
            }
        }
    }

    /// [`Self::key`] for a stream that can lose key-ups (the Windows hook
    /// under UIPI, a macOS tap under secure input): before a key-down is
    /// matched, every key believed down that `still_down` says is up is
    /// released first. A key-up lost while nothing was pressed would otherwise
    /// stay "held" and refuse every chord without it (matching is exact on
    /// modifiers) until the user pressed and released that key again.
    #[cfg_attr(not(any(windows, target_os = "macos", test)), allow(dead_code))]
    pub(crate) fn key_reconciled(
        &mut self,
        code: KeyCode,
        down: bool,
        still_down: impl Fn(KeyCode) -> bool,
        out: &mut Vec<(BindingKey, HotkeyState)>,
    ) {
        if down {
            let stale: Vec<KeyCode> = self
                .down
                .iter()
                .copied()
                .filter(|&k| k != code && !still_down(k))
                .collect();
            for k in stale {
                self.key(k, false, out);
            }
        }
        self.key(code, down, out);
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
        for b in &mut self.bindings {
            if b.pressed {
                b.pressed = false;
                out.push((b.key, HotkeyState::Released));
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
        // Left Ctrl presses nothing, and neither does Right Ctrl pressed while
        // C is already held. (Right Ctrl *then* C does press it, and reports
        // the release `chorded`: see the next test.)
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

    /// Right Ctrl then C (the natural copy order) presses a `RightCtrl`
    /// binding at once — hold-to-talk cannot wait — and its release says a
    /// chord happened, so a toggle app can ignore it. Lock keys do not count.
    #[test]
    fn a_key_pressed_during_a_modifier_only_binding_marks_it_chorded() {
        let mut mm = m(&[(7, "RightCtrl")]);
        let out = run(
            &mut mm,
            &[
                (ControlRight, true),
                (KeyC, true),
                (KeyC, false),
                (ControlRight, false),
            ],
        );
        assert_eq!(out, vec![(7, Pressed), (7, Released)]);
        assert!(mm.chorded(7));
        run(&mut mm, &[(ControlRight, true)]);
        assert!(!mm.chorded(7), "a new press starts clean");
        run(&mut mm, &[(CapsLock, true), (ControlRight, false)]);
        assert!(!mm.chorded(7), "a lock key is not a chord");
    }

    /// A key-up the stream lost is dropped at the next key-down by a backend
    /// that can ask the OS (`key_reconciled`).
    #[test]
    fn a_reconciled_stream_drops_a_lost_key_up() {
        let mut mm = m(&[(1, "Ctrl+Space")]);
        run(&mut mm, &[(AltLeft, true)]); // its key-up is lost
        let mut out = Vec::new();
        let alt_is_up = |k: KeyCode| k != AltLeft;
        for (k, d) in [(ControlLeft, true), (Space, true)] {
            mm.key_reconciled(k, d, alt_is_up, &mut out);
        }
        assert_eq!(out, vec![(1, Pressed)]);
        // The plain `key` would have kept Alt and refused the chord.
        let mut plain = m(&[(1, "Ctrl+Space")]);
        assert!(
            run(
                &mut plain,
                &[(AltLeft, true), (ControlLeft, true), (Space, true)]
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
