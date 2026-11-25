//! Translating winit's input into rinch's: pointer sources, buttons, wheels, keys and modifiers.

use super::*;

mod pointer;
pub use pointer::*;

/// Windows' `VK_PACKET`: the virtual key of a `KEYEVENTF_UNICODE` injection.
const VK_PACKET: u16 = 0xE7;

/// What the shell does with one winit key event
/// ([`RinchRuntime::route_key_event`]).
#[derive(Debug)]
pub(super) enum KeyRoute {
    /// Hand the app this event.
    Event(PlatformEvent),
    /// A menu chord ran and consumed the key: redraw, and nothing else.
    ShortcutRan,
    /// Nothing to do.
    Nothing,
}

impl RinchRuntime {
    /// The DOM-style key value a winit **logical** key spells — what feeds
    /// `PlatformEvent::KeyDown::logical_key` (and `KeyUp`'s), see the contract
    /// there. winit already speaks the browser's language, so this is nearly a
    /// transcription:
    ///
    /// - `Key::Character` is the layout-produced string, **verbatim**. Case is
    ///   preserved deliberately: `KeyboardEvent.key` in a browser is
    ///   case-*accurate* — measured, Chromium reports `"A"` for Shift+A and
    ///   `"S"` for Ctrl+Shift+S, on the press **and** the release. The old
    ///   lowercased-single-letter form made a release disagree with its own
    ///   press for every capital (a press spelled itself from `text`, which
    ///   kept the capital; a release has no text), which breaks the one thing
    ///   releases exist for — pairing (issue #337). Consumers that want a
    ///   case-insensitive identity fold at the comparison site, as
    ///   `editor_key_binding` does. The spacebar arrives here too, as `" "`,
    ///   the spec's own spelling.
    /// - `Key::Named` is `keyboard_types::NamedKey`, the W3C key-values enum;
    ///   its `Display` **is** the spec string (`"Enter"`, `"ArrowLeft"`,
    ///   `"Shift"`). winit's backends already fold the platform quirks the
    ///   browser folds — X11/Wayland's Super keysym is reported as
    ///   `NamedKey::Meta` "because browsers do" (winit-common's xkb keymap) —
    ///   so no respelling happens here.
    /// - A dead key is `"Dead"`, the value every browser reports for one.
    /// - `Key::Unidentified` is `None`, not the spec's `"Unidentified"`: it
    ///   carries only a native keysym rinch cannot spell, and `None` is what
    ///   lets the consumer fall back to the physical `key` — two different
    ///   unknown keys must not pair with each other by a shared placeholder
    ///   string.
    pub(super) fn winit_logical_key_str(key: &winit::keyboard::Key) -> Option<String> {
        use winit::keyboard::Key;
        match key {
            Key::Character(s) => Some(s.to_string()),
            Key::Named(n) => Some(n.to_string()),
            Key::Dead(_) => Some("Dead".to_string()),
            Key::Unidentified(_) => None,
        }
    }

    /// The [`PlatformEvent::KeyDown`] a winit press translates to.
    ///
    /// Split out of the `WindowEvent::KeyboardInput` arm so it can be tested at
    /// all: `window_event` needs a live `ActiveEventLoop` and a real window, and
    /// the only other statement in that arm is the menu-shortcut consume.
    ///
    /// What that buys is the **one** pin on this backend's `repeat` answer.
    /// Every consumer fixture supplies its own [`KeyRepeat`], so inverting the
    /// mapping here — desktop refusing every fresh press and activating on
    /// every auto-repeat, i.e. #463 made worse than before it — was invisible
    /// to the entire runtime suite (measured: 602 passed, 0 failed, in the
    /// adversarial review of PR #796). The assertion in
    /// `key_repeat_translation` is trivial by construction; the point is that
    /// there was nothing at all.
    pub(super) fn key_down_from_winit(
        key_code: winit::keyboard::KeyCode,
        logical_key: &winit::keyboard::Key,
        text: Option<&str>,
        repeat: bool,
        modifiers: Modifiers,
    ) -> PlatformEvent {
        PlatformEvent::KeyDown {
            key: Self::translate_key(key_code),
            // The layout-mapped key value (so Mod+letter follows the keycap, not
            // the physical position) — winit's logical key carries it even when a
            // modifier suppresses `text`.
            logical_key: Self::winit_logical_key_str(logical_key),
            text: text.map(str::to_string),
            modifiers,
            // winit's `repeat` is `true` if and only if the OS made this press
            // by holding the key down, so desktop never has to infer it from a
            // release that may never arrive (#463).
            repeat: if repeat {
                KeyRepeat::Repeat
            } else {
                KeyRepeat::Fresh
            },
        }
    }

    /// What the shell does with one winit `KeyboardInput` for `app`, whose
    /// window's modifiers are `mods` and whose pointer `pointer_locked` says
    /// whether it holds a lock: the whole of that arm of `window_event` bar
    /// the redraw, so a fixture can drive it with a real winit [`KeyEvent`]
    /// (`window_event` itself needs a live event loop).
    ///
    /// - Text no key typed ([`Self::is_text_only_key`], #2848) is its text
    ///   alone ([`Self::text_only_key_down`]), and its release is nothing.
    /// - A press tries the menu chords first; one that ran consumes the key.
    ///   A chord with no Ctrl/Cmd/Alt leaves its key to a focused text field
    ///   (#1169). `key_without_modifiers` is the layout's character for this
    ///   physical key, ignoring Shift — issue #1170's letter chords match it
    ///   before falling back to the physical key. Escape while the pointer is
    ///   locked is the lock's (#2092): `handle_event` ends it and consumes
    ///   the key, so it is no chord.
    /// - Any other key winit could not place is nothing, as before.
    ///
    /// [`KeyEvent`]: winit::event::KeyEvent
    pub(super) fn route_key_event(
        app: &RinchApp,
        event: &winit::event::KeyEvent,
        mods: Modifiers,
        pointer_locked: bool,
    ) -> KeyRoute {
        use winit::keyboard::{KeyCode as WK, PhysicalKey};
        if Self::is_text_only_key(event) {
            return Self::text_only_key_down(event).map_or(KeyRoute::Nothing, KeyRoute::Event);
        }
        let PhysicalKey::Code(key_code) = event.physical_key else {
            return KeyRoute::Nothing;
        };
        if event.state == ElementState::Released {
            return KeyRoute::Event(PlatformEvent::KeyUp {
                key: Self::translate_key(key_code),
                // winit's `KeyEvent` is one struct for both states and fills
                // this on each; only `text` is press-gated. Forwarding it is
                // what lets a release be spelled by the same rule as its press
                // (issue #337) — the shell was simply dropping it.
                logical_key: Self::winit_logical_key_str(&event.logical_key),
                modifiers: mods,
            });
        }
        let text = event.text.as_deref();
        let lock_escape = key_code == WK::Escape && pointer_locked;
        if !lock_escape && app.try_menu_shortcut(mods, key_code, &event.key_without_modifiers, text)
        {
            // The chord never reaches `handle_event`, which reports every other
            // fresh press to the app-wide key listeners (#3062): a stale record
            // of this key is dropped here, so the chord's release is not theirs.
            if !event.repeat {
                rinch_core::note_fresh_key_press(&format!("{:?}", Self::translate_key(key_code)));
            }
            return KeyRoute::ShortcutRan;
        }
        // Under Ctrl+Alt the text survives only when AltGr typed it (Windows
        // reports AltGr as Ctrl+Alt; #934), so the app can insert it without
        // knowing the layout.
        let unmodified = match &event.key_without_modifiers {
            winit::keyboard::Key::Character(s) => Some(s.as_str()),
            _ => None,
        };
        let text = rinch_platform::alt_graph_filtered_text(
            rinch_platform::alt_graph_is_ctrl_alt(),
            mods,
            text,
            unmodified,
        );
        KeyRoute::Event(Self::key_down_from_winit(
            key_code,
            &event.logical_key,
            text,
            event.repeat,
            mods,
        ))
    }

    /// Whether a winit key event is **text that no key typed** (#2848): a
    /// Windows `VK_PACKET` — what `SendInput` with `KEYEVENTF_UNICODE` makes,
    /// as dictation tools, on-screen keyboards and remote-input apps send
    /// text — or a key whose physical position winit could not identify.
    ///
    /// Measured on Windows 11 with beardictate's injector, every packet press
    /// and release has the physical key `Unidentified(Windows(0))`: the
    /// scan-code field of the message is 0. The shell's key arms matched only
    /// `PhysicalKey::Code`, so all of them were dropped. A packet is also
    /// recognised by its logical key, which winit-win32 reports as
    /// `Key::Unidentified(NativeKey::Windows(VK_PACKET))`. That way one whose
    /// scan-code field is not 0 is not typed as whatever key has that scan
    /// code. The scan-code field comes from the injector's `wScan`, which for
    /// `KEYEVENTF_UNICODE` is the character: `" "` would be 0x20, the D key.
    pub(super) fn is_text_only_key(event: &winit::event::KeyEvent) -> bool {
        use winit::keyboard::{Key, NativeKey, PhysicalKey};
        matches!(event.physical_key, PhysicalKey::Unidentified(_))
            || matches!(
                event.logical_key,
                Key::Unidentified(NativeKey::Windows(VK_PACKET))
            )
    }

    /// The [`PlatformEvent::KeyDown`] a text-only press
    /// ([`Self::is_text_only_key`]) becomes, or `None` (a release, or a press
    /// carrying no printable text).
    ///
    /// It is the press Android's soft keyboard sends for committed text
    /// (`apply_ime_action`): [`KeyCode::Other`], no logical key, no
    /// modifiers, a fresh press. So it takes the same path as any typed
    /// character into the focused `<input>` or editor (the editor's input
    /// rules included), and nothing that reads a key: no menu or built-in
    /// shortcut, editor key binding or activation matches `Other` with no
    /// logical key, and with no modifiers a Ctrl held while a dictation tool
    /// types (a push-to-talk chord) neither suppresses the text nor makes it a
    /// chord. Under that Ctrl winit-win32 leaves `text` empty for a
    /// `VK_PACKET` (it re-derives Ctrl text from the layout, which has no
    /// character for the packet key), so the text then comes from
    /// `text_with_all_modifiers`, which holds the `WM_CHAR` unit as sent.
    /// Control characters are dropped: a line break or tab is a key, and an
    /// injector that means one sends the key.
    pub(super) fn text_only_key_down(event: &winit::event::KeyEvent) -> Option<PlatformEvent> {
        if event.state != ElementState::Pressed {
            return None;
        }
        let text: String = event
            .text
            .as_deref()
            .filter(|t| !t.is_empty())
            .or(event.text_with_all_modifiers.as_deref())?
            .chars()
            .filter(|c| !c.is_control())
            .collect();
        (!text.is_empty()).then(|| PlatformEvent::KeyDown {
            key: KeyCode::Other,
            logical_key: None,
            text: Some(text),
            modifiers: Modifiers::default(),
            repeat: KeyRepeat::Fresh,
        })
    }

    /// Translate a winit KeyCode to a platform KeyCode.
    pub(super) fn translate_key(key_code: winit::keyboard::KeyCode) -> KeyCode {
        use winit::keyboard::KeyCode as WK;
        match key_code {
            WK::ArrowLeft => KeyCode::ArrowLeft,
            WK::ArrowRight => KeyCode::ArrowRight,
            WK::ArrowUp => KeyCode::ArrowUp,
            WK::ArrowDown => KeyCode::ArrowDown,
            WK::Home => KeyCode::Home,
            WK::End => KeyCode::End,
            WK::PageUp => KeyCode::PageUp,
            WK::PageDown => KeyCode::PageDown,
            WK::Enter | WK::NumpadEnter => KeyCode::Enter,
            WK::Backspace => KeyCode::Backspace,
            WK::Delete => KeyCode::Delete,
            WK::Tab => KeyCode::Tab,
            WK::Escape => KeyCode::Escape,
            WK::Space => KeyCode::Space,
            WK::KeyA => KeyCode::KeyA,
            WK::KeyB => KeyCode::KeyB,
            WK::KeyC => KeyCode::KeyC,
            WK::KeyD => KeyCode::KeyD,
            WK::KeyE => KeyCode::KeyE,
            WK::KeyF => KeyCode::KeyF,
            WK::KeyG => KeyCode::KeyG,
            WK::KeyH => KeyCode::KeyH,
            WK::KeyI => KeyCode::KeyI,
            WK::KeyJ => KeyCode::KeyJ,
            WK::KeyK => KeyCode::KeyK,
            WK::KeyL => KeyCode::KeyL,
            WK::KeyM => KeyCode::KeyM,
            WK::KeyN => KeyCode::KeyN,
            WK::KeyO => KeyCode::KeyO,
            WK::KeyP => KeyCode::KeyP,
            WK::KeyQ => KeyCode::KeyQ,
            WK::KeyR => KeyCode::KeyR,
            WK::KeyS => KeyCode::KeyS,
            WK::KeyT => KeyCode::KeyT,
            WK::KeyU => KeyCode::KeyU,
            WK::KeyV => KeyCode::KeyV,
            WK::KeyW => KeyCode::KeyW,
            WK::KeyX => KeyCode::KeyX,
            WK::KeyY => KeyCode::KeyY,
            WK::KeyZ => KeyCode::KeyZ,
            WK::Digit0 => KeyCode::Digit0,
            WK::Digit1 => KeyCode::Digit1,
            WK::Digit2 => KeyCode::Digit2,
            WK::Digit3 => KeyCode::Digit3,
            WK::Digit4 => KeyCode::Digit4,
            WK::Digit5 => KeyCode::Digit5,
            WK::Digit6 => KeyCode::Digit6,
            WK::Digit7 => KeyCode::Digit7,
            WK::Digit8 => KeyCode::Digit8,
            WK::Digit9 => KeyCode::Digit9,
            WK::F1 => KeyCode::F1,
            WK::F2 => KeyCode::F2,
            WK::F3 => KeyCode::F3,
            WK::F4 => KeyCode::F4,
            WK::F5 => KeyCode::F5,
            WK::F6 => KeyCode::F6,
            WK::F7 => KeyCode::F7,
            WK::F8 => KeyCode::F8,
            WK::F9 => KeyCode::F9,
            WK::F10 => KeyCode::F10,
            WK::F11 => KeyCode::F11,
            WK::F12 => KeyCode::F12,
            WK::Equal => KeyCode::Equal,
            WK::Minus => KeyCode::Minus,
            WK::ShiftLeft => KeyCode::ShiftLeft,
            WK::ShiftRight => KeyCode::ShiftRight,
            WK::ControlLeft => KeyCode::ControlLeft,
            WK::ControlRight => KeyCode::ControlRight,
            WK::AltLeft => KeyCode::AltLeft,
            WK::AltRight => KeyCode::AltRight,
            _ => KeyCode::Other,
        }
    }
}

impl ShellWindow {
    /// Translate winit modifier state to platform Modifiers.
    pub(super) fn translate_modifiers(&self) -> Modifiers {
        Modifiers {
            shift: self.modifiers.shift_key(),
            ctrl: self.modifiers.control_key(),
            alt: self.modifiers.alt_key(),
            meta: self.modifiers.meta_key(),
        }
    }
}

#[cfg(test)]
mod logical_key_spelling;

/// The desktop backend's `KeyRepeat` answer (issue #463).
///
/// The one producer pin on this backend. Every `key_repeat_tests` fixture hands
/// the runtime a `KeyRepeat` of its own, so nothing there can tell whether the
/// shell reports winit's flag, its negation, or a constant — inverting it
/// survived all 602 tests of `cargo test -p rinch --lib --features debug` in the
/// adversarial review of PR #796. That is the hole these two assertions close,
/// and it is worth knowing they are the *only* thing closing it.
#[cfg(test)]
mod key_repeat_translation;

/// Text no key typed (#2848): a Windows `VK_PACKET` from `SendInput` with
/// `KEYEVENTF_UNICODE`, and a key winit could not place, as the winit-win32
/// backend reports them. Each press is built the way winit-win32 builds it —
/// the logical key `Unidentified(Windows(VK_PACKET))`, the physical key from
/// the scan-code field. They are sampled off the fixed point: `Unidentified`
/// (what Windows 11 sends, measured) beside physical keys that are real keys
/// (`KeyD`, `Space`), and text under Ctrl beside bare text.
#[cfg(test)]
mod text_only_key_translation;

/// winit's pointer sources as a render surface hears them
/// (`SurfacePointer`): which device, which id, how hard.
#[cfg(test)]
mod pointer_source_tests;
