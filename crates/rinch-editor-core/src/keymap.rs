//! Platform-agnostic key bindings.
//!
//! The core speaks an abstract keyboard vocabulary — [`Key`] + [`Modifiers`] —
//! that carries no `winit`/`web_sys` types. A platform view translates its native
//! key event into a [`KeyBinding`], looks it up in the aggregated [`Keymap`], and
//! runs the bound command (design §5; salvages the `normalize_key` *logic* from the
//! old `shortcuts.rs` but consumes events at the platform edge, not here).
//!
//! ## "Mod" / the primary accelerator
//!
//! Bindings are written with `Mod` (e.g. `"Mod-b"`) meaning *the platform's primary
//! accelerator* — Ctrl on Windows/Linux, Cmd on macOS. The core collapses
//! Ctrl/Cmd/Meta/Mod into a single [`Modifiers::primary`] flag; the platform view
//! decides which physical modifier maps to `primary` for the host OS. This keeps
//! one binding table correct on every platform.

use std::collections::HashMap;

/// An abstract key — a named editing key or a printable character (lower-cased;
/// case is carried by [`Modifiers::shift`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    /// A printable character, lower-cased (`'b'` for both `b` and `Shift-B`).
    Char(char),
    /// Return / Enter.
    Enter,
    /// Backspace.
    Backspace,
    /// Forward delete.
    Delete,
    /// Tab.
    Tab,
    /// Escape.
    Escape,
    /// Space (distinct from `Char(' ')` so `"Space"` parses).
    Space,
    /// Left arrow.
    ArrowLeft,
    /// Right arrow.
    ArrowRight,
    /// Up arrow.
    ArrowUp,
    /// Down arrow.
    ArrowDown,
    /// Home.
    Home,
    /// End.
    End,
    /// Page up.
    PageUp,
    /// Page down.
    PageDown,
}

impl Key {
    /// Parse a key token (`"Enter"`, `"ArrowLeft"`, `"b"`, `" "`). Names are
    /// case-insensitive; a single character becomes a lower-cased [`Key::Char`].
    pub fn parse(s: &str) -> Option<Key> {
        let named = match s.to_ascii_lowercase().as_str() {
            "enter" | "return" => Some(Key::Enter),
            "backspace" => Some(Key::Backspace),
            "delete" | "del" => Some(Key::Delete),
            "tab" => Some(Key::Tab),
            "escape" | "esc" => Some(Key::Escape),
            "space" => Some(Key::Space),
            "arrowleft" | "left" => Some(Key::ArrowLeft),
            "arrowright" | "right" => Some(Key::ArrowRight),
            "arrowup" | "up" => Some(Key::ArrowUp),
            "arrowdown" | "down" => Some(Key::ArrowDown),
            "home" => Some(Key::Home),
            "end" => Some(Key::End),
            "pageup" => Some(Key::PageUp),
            "pagedown" => Some(Key::PageDown),
            _ => None,
        };
        if named.is_some() {
            return named;
        }
        if s == " " {
            return Some(Key::Space);
        }
        let mut chars = s.chars();
        let c = chars.next()?;
        if chars.next().is_some() {
            return None; // more than one char and not a known name
        }
        Some(Key::Char(c.to_ascii_lowercase()))
    }
}

/// Modifier flags. `primary` is the OS accelerator (Ctrl / Cmd); `shift` and `alt`
/// are literal.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Modifiers {
    /// The platform's primary accelerator (Ctrl on Windows/Linux, Cmd on macOS).
    pub primary: bool,
    /// Shift.
    pub shift: bool,
    /// Alt / Option.
    pub alt: bool,
}

/// A key plus its modifiers — the lookup key for a [`Keymap`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct KeyBinding {
    /// The key.
    pub key: Key,
    /// The modifiers held.
    pub mods: Modifiers,
}

impl KeyBinding {
    /// A binding from a key and modifiers.
    pub fn new(key: Key, mods: Modifiers) -> KeyBinding {
        KeyBinding { key, mods }
    }

    /// Parse a binding string like `"Mod-b"`, `"Shift-Enter"`, `"Mod-Shift-z"`,
    /// `"Mod--"` (primary + minus). Modifier tokens (case-insensitive): `Mod`,
    /// `Primary`, `Ctrl`/`Control`, `Cmd`/`Meta`/`Super` → `primary`; `Shift`;
    /// `Alt`/`Option`. The final token is the key.
    pub fn parse(s: &str) -> Option<KeyBinding> {
        let mut mods = Modifiers::default();
        let mut rest = s;
        loop {
            let Some(idx) = rest.find('-') else {
                return Some(KeyBinding::new(Key::parse(rest)?, mods));
            };
            let tok = &rest[..idx];
            let after = &rest[idx + 1..];
            match tok.to_ascii_lowercase().as_str() {
                "mod" | "primary" | "ctrl" | "control" | "cmd" | "meta" | "super" => {
                    mods.primary = true;
                    rest = after;
                }
                "shift" => {
                    mods.shift = true;
                    rest = after;
                }
                "alt" | "option" => {
                    mods.alt = true;
                    rest = after;
                }
                // Not a modifier token → `rest` (which still includes any internal
                // '-', e.g. the key "-") is the key.
                _ => return Some(KeyBinding::new(Key::parse(rest)?, mods)),
            }
        }
    }
}

/// A binding → command-name lookup table, aggregated from all plugins.
#[derive(Debug, Default, Clone)]
pub struct Keymap {
    map: HashMap<KeyBinding, String>,
}

impl Keymap {
    /// An empty keymap.
    pub fn new() -> Keymap {
        Keymap {
            map: HashMap::new(),
        }
    }

    /// Bind `binding` to the command named `command`. Later bindings override
    /// earlier ones for the same key.
    pub fn add(&mut self, binding: KeyBinding, command: impl Into<String>) {
        self.map.insert(binding, command.into());
    }

    /// Bind a parsed string (`"Mod-b"`) to a command; ignored if the string does
    /// not parse.
    pub fn bind(&mut self, binding: &str, command: impl Into<String>) {
        if let Some(b) = KeyBinding::parse(binding) {
            self.add(b, command);
        }
    }

    /// The command name bound to `binding`, if any.
    pub fn command_for(&self, binding: &KeyBinding) -> Option<&str> {
        self.map.get(binding).map(String::as_str)
    }

    /// Number of bindings.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// True if no bindings.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_mod_letter() {
        let b = KeyBinding::parse("Mod-b").unwrap();
        assert_eq!(b.key, Key::Char('b'));
        assert!(b.mods.primary);
        assert!(!b.mods.shift && !b.mods.alt);
    }

    #[test]
    fn parses_named_and_combos() {
        assert_eq!(KeyBinding::parse("Shift-Enter").unwrap().key, Key::Enter);
        let z = KeyBinding::parse("Mod-Shift-z").unwrap();
        assert_eq!(z.key, Key::Char('z'));
        assert!(z.mods.primary && z.mods.shift);
        // case-insensitive modifiers + uppercase letter folds to lower
        let b = KeyBinding::parse("mod-B").unwrap();
        assert_eq!(b.key, Key::Char('b'));
    }

    #[test]
    fn parses_minus_key() {
        let b = KeyBinding::parse("Mod--").unwrap();
        assert_eq!(b.key, Key::Char('-'));
        assert!(b.mods.primary);
    }

    #[test]
    fn ctrl_cmd_meta_all_map_to_primary() {
        for s in ["Ctrl-a", "Cmd-a", "Meta-a", "Primary-a"] {
            assert!(KeyBinding::parse(s).unwrap().mods.primary, "{s}");
        }
    }

    #[test]
    fn keymap_lookup() {
        let mut km = Keymap::new();
        km.bind("Mod-b", "toggleBold");
        let b = KeyBinding::parse("Mod-b").unwrap();
        assert_eq!(km.command_for(&b), Some("toggleBold"));
        let other = KeyBinding::parse("Mod-i").unwrap();
        assert_eq!(km.command_for(&other), None);
    }
}
