//! Keys named the way the W3C `KeyboardEvent.key` spells them ("Enter",
//! "ArrowLeft", "a"), turned into the [`PlatformEvent::KeyDown`] /
//! [`PlatformEvent::KeyUp`] a physical keystroke delivers.
//!
//! Shared by the two callers that synthesize keystrokes rather than receive
//! them from winit: the debug interface's `key_press`/`key_down`/`key_up`/
//! `type_text` (`debug_commands.rs`) and the public test harness
//! (`crate::testing`, issue #2553). Both go through
//! [`RinchApp::dispatch_named_key_down`], which checks the menu shortcuts first
//! exactly as the winit `KeyboardInput` arm does (issue #533), so neither can
//! drift from the other or from a real keyboard.

use super::*;

/// Map a printable character to a physical `KeyCode` for synthesizing keystrokes
/// (the `text` field carries the actual character). Characters without a
/// dedicated keycode map to `KeyCode::Other` — exactly how real hardware
/// delivers punctuation (`shell/rinch_runtime/winit_input.rs` translates unlisted winit
/// keys to `Other`) — so the keyboard hook's key string falls through to the
/// `text` field instead of masquerading as a named key. Space keeps its
/// explicit arm: an injected `' '` must still read as the spacebar —
/// `code = "Space"`, `key = " "` — matching a physical spacebar press (issue
/// #151; the `key` spelling is #1161's).
pub(crate) fn char_to_keycode(c: char) -> KeyCode {
    match c.to_ascii_lowercase() {
        ' ' => KeyCode::Space,
        'a' => KeyCode::KeyA,
        'b' => KeyCode::KeyB,
        'c' => KeyCode::KeyC,
        'd' => KeyCode::KeyD,
        'e' => KeyCode::KeyE,
        'f' => KeyCode::KeyF,
        'g' => KeyCode::KeyG,
        'h' => KeyCode::KeyH,
        'i' => KeyCode::KeyI,
        'j' => KeyCode::KeyJ,
        'k' => KeyCode::KeyK,
        'l' => KeyCode::KeyL,
        'm' => KeyCode::KeyM,
        'n' => KeyCode::KeyN,
        'o' => KeyCode::KeyO,
        'p' => KeyCode::KeyP,
        'q' => KeyCode::KeyQ,
        'r' => KeyCode::KeyR,
        's' => KeyCode::KeyS,
        't' => KeyCode::KeyT,
        'u' => KeyCode::KeyU,
        'v' => KeyCode::KeyV,
        'w' => KeyCode::KeyW,
        'x' => KeyCode::KeyX,
        'y' => KeyCode::KeyY,
        'z' => KeyCode::KeyZ,
        '0' => KeyCode::Digit0,
        '1' => KeyCode::Digit1,
        '2' => KeyCode::Digit2,
        '3' => KeyCode::Digit3,
        '4' => KeyCode::Digit4,
        '5' => KeyCode::Digit5,
        '6' => KeyCode::Digit6,
        '7' => KeyCode::Digit7,
        '8' => KeyCode::Digit8,
        '9' => KeyCode::Digit9,
        _ => KeyCode::Other,
    }
}

/// Map a debug `key_press` key-name string to a `KeyCode`. Single-character
/// names go through [`char_to_keycode`] (punctuation → `KeyCode::Other`, with
/// the character delivered via the event's `text` field); unknown multi-char
/// names return `None` so the caller can fail loud instead of synthesizing a
/// silent no-text `Other` press.
pub(crate) fn keyname_to_keycode(key: &str) -> Option<KeyCode> {
    Some(match key {
        "ArrowLeft" => KeyCode::ArrowLeft,
        "ArrowRight" => KeyCode::ArrowRight,
        "ArrowUp" => KeyCode::ArrowUp,
        "ArrowDown" => KeyCode::ArrowDown,
        "Home" => KeyCode::Home,
        "End" => KeyCode::End,
        "PageUp" => KeyCode::PageUp,
        "PageDown" => KeyCode::PageDown,
        "Enter" => KeyCode::Enter,
        "Backspace" => KeyCode::Backspace,
        "Delete" => KeyCode::Delete,
        "Tab" => KeyCode::Tab,
        "Escape" => KeyCode::Escape,
        "Space" => KeyCode::Space,
        "F12" => KeyCode::F12,
        k if k.chars().count() == 1 => char_to_keycode(k.chars().next().unwrap()),
        _ => return None,
    })
}

/// The W3C `KeyboardEvent.code` name for a debug `key_press` key name — the
/// same string [`crate::menu::match_shortcut_code`] (and the winit shell's own
/// `KeyboardInput` translation) key menu-chord matching off of.
///
/// Issue #533: `key_press` used to build a `PlatformEvent::KeyDown` directly
/// and hand it to `handle_event`, entering the pipeline *downstream* of the
/// menu-shortcut check the real winit `WindowEvent::KeyboardInput` arm makes
/// first — so an injected `Ctrl+D` never reached a registered "Ctrl+D" menu
/// item, whether or not the app's real shortcut worked. Mirrors
/// [`keyname_to_keycode`]'s table exactly (named keys verbatim, a single
/// character through [`char_to_w3c_code`]) so the two stay in lockstep; a
/// punctuation character has no entry here either, since `char_to_keycode`
/// never gave it a dedicated `KeyCode` and a debug `key_press` could not
/// reach a punctuation-bound chord before this change and still cannot.
pub(crate) fn keyname_to_w3c_code(key: &str) -> Option<&'static str> {
    Some(match key {
        "ArrowLeft" => "ArrowLeft",
        "ArrowRight" => "ArrowRight",
        "ArrowUp" => "ArrowUp",
        "ArrowDown" => "ArrowDown",
        "Home" => "Home",
        "End" => "End",
        "PageUp" => "PageUp",
        "PageDown" => "PageDown",
        "Enter" => "Enter",
        "Backspace" => "Backspace",
        "Delete" => "Delete",
        "Tab" => "Tab",
        "Escape" => "Escape",
        "Space" => "Space",
        "F12" => "F12",
        k if k.chars().count() == 1 => char_to_w3c_code(k.chars().next().unwrap())?,
        _ => return None,
    })
}

/// The W3C `code` name for a single character, mirroring [`char_to_keycode`]'s
/// table one-for-one (and returning `None` exactly where that one falls back
/// to `KeyCode::Other`).
pub(crate) fn char_to_w3c_code(c: char) -> Option<&'static str> {
    Some(match c.to_ascii_lowercase() {
        ' ' => "Space",
        'a' => "KeyA",
        'b' => "KeyB",
        'c' => "KeyC",
        'd' => "KeyD",
        'e' => "KeyE",
        'f' => "KeyF",
        'g' => "KeyG",
        'h' => "KeyH",
        'i' => "KeyI",
        'j' => "KeyJ",
        'k' => "KeyK",
        'l' => "KeyL",
        'm' => "KeyM",
        'n' => "KeyN",
        'o' => "KeyO",
        'p' => "KeyP",
        'q' => "KeyQ",
        'r' => "KeyR",
        's' => "KeyS",
        't' => "KeyT",
        'u' => "KeyU",
        'v' => "KeyV",
        'w' => "KeyW",
        'x' => "KeyX",
        'y' => "KeyY",
        'z' => "KeyZ",
        '0' => "Digit0",
        '1' => "Digit1",
        '2' => "Digit2",
        '3' => "Digit3",
        '4' => "Digit4",
        '5' => "Digit5",
        '6' => "Digit6",
        '7' => "Digit7",
        '8' => "Digit8",
        '9' => "Digit9",
        _ => return None,
    })
}

/// The single lowercase ASCII letter a debug `key_press`'s `key` name spells,
/// or `None` for anything else — the debug equivalent of winit's
/// `key_without_modifiers` / a browser's lowercased `event.key()` that
/// [`crate::menu::chord_key_matches`] reads for issue #1170's layout-character
/// matching. MCP has no layout to consult, so (as `logical_key` below already
/// assumes) the key name IS the character: `key_press(key: "D", shift: true)`
/// types the same letter a QWERTY `Shift+D` does.
pub(crate) fn keyname_typed_letter(key: &str) -> Option<char> {
    let mut chars = key.chars();
    let c = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    c.is_ascii_alphabetic().then(|| c.to_ascii_lowercase())
}

/// The `logical_key` a debug key name resolves to, shared by
/// `key_press`/`key_down` (which also derive `text`, press-only) and
/// `key_up` (issue #485): a release must spell the same `logical_key` as its
/// press, or a consumer pairing them by that string (issue #337) never sees
/// them match. A single-character name is its own key value, case and all
/// (MCP has no layout to consult); a named key stays `None` — its `KeyCode`
/// spells it, and fabricating the name here would just shadow that table.
pub(crate) fn debug_key_logical_key(key: &str) -> Option<String> {
    let mut it = key.chars();
    match (it.next(), it.next()) {
        (Some(c), None) if !c.is_control() => Some(c.to_string()),
        _ => None,
    }
}

/// What [`RinchApp::dispatch_named_key_down`] made of a key name.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum NamedKeyDown {
    /// A `KeyDown` went to the app. The release a caller owes it is a
    /// `KeyUp` of this key, logical key and modifiers.
    Dispatched {
        key: KeyCode,
        logical_key: Option<String>,
        modifiers: Modifiers,
    },
    /// A menu shortcut took the chord before the app saw it: no `KeyDown` was
    /// dispatched, so nothing is armed and no release is owed — as with a
    /// real keystroke a menu owns.
    MenuShortcut,
    /// Not a key name [`keyname_to_keycode`] knows.
    UnknownKey,
}

impl RinchApp {
    /// Press the key `key` names with `modifiers` held: check the menu
    /// shortcuts first — exactly as the real winit `WindowEvent::KeyboardInput`
    /// arm does in `rinch_runtime.rs` (issue #533) — and otherwise dispatch a
    /// `PlatformEvent::KeyDown` through `handle_event`. The actions it asked
    /// for are appended to `actions`.
    ///
    /// `text_target_holds_keyboard` lets a modifier-less chord yield to a
    /// focused text field (#1169), same as a real keystroke; it is
    /// desktop-only machinery, so an `android`/`embed` build without `desktop`
    /// never yields there.
    pub(crate) fn dispatch_named_key_down(
        &mut self,
        key: &str,
        modifiers: Modifiers,
        actions: &mut Vec<AppAction>,
        window_size: (u32, u32),
        scale_factor: f64,
    ) -> NamedKeyDown {
        let Modifiers {
            shift,
            ctrl,
            alt,
            meta,
        } = modifiers;
        // Unknown multi-char key names fail loud — a silent no-text `Other`
        // press would be indistinguishable from a dead key (#151).
        let Some(key_code) = keyname_to_keycode(key) else {
            return NamedKeyDown::UnknownKey;
        };
        #[cfg(feature = "desktop")]
        let text_focus = self.text_target_holds_keyboard();
        #[cfg(not(feature = "desktop"))]
        let text_focus = false;
        if let Some(code) = keyname_to_w3c_code(key) {
            let yields =
                text_focus && crate::menu::chord_yields_to_text_input(ctrl, meta, alt, code);
            if !yields {
                let typed_letter = keyname_typed_letter(key);
                if crate::menu::match_shortcut_code(ctrl, meta, alt, shift, code, typed_letter) {
                    return NamedKeyDown::MenuShortcut;
                }
            }
        }
        let text = match key {
            "Enter" => Some("\n".to_string()),
            k if k.chars().count() == 1 => Some(k.to_string()),
            _ => None,
        };
        let logical_key = debug_key_logical_key(key);
        actions.extend(self.handle_event(
            PlatformEvent::KeyDown {
                key: key_code,
                logical_key: logical_key.clone(),
                text,
                modifiers,
                repeat: rinch_platform::KeyRepeat::Fresh,
            },
            window_size,
            scale_factor,
        ));
        NamedKeyDown::Dispatched {
            key: key_code,
            logical_key,
            modifiers,
        }
    }
}
