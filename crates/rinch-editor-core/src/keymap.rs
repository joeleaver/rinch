//! Platform-agnostic key bindings.
//!
//! The core speaks an abstract keyboard vocabulary — [`Key`] + [`Modifiers`] —
//! that carries no `winit`/`web_sys` types. A platform view translates its native
//! key event into a [`KeyBinding`], looks it up in the aggregated [`Keymap`], and
//! runs the bound command (design §5; salvages the `normalize_key` *logic* from the
//! old `shortcuts.rs` but consumes events at the platform edge, not here).
//!
//! ## Which keys a binding consumes
//!
//! A binding consumes its key whether or not its command applies: the platform
//! never passes on a chord the editor advertises (Ctrl+Shift+R on a paragraph
//! that is already right-aligned must not reach the browser's hard reload). A
//! binding can instead be declared **fall-through** ([`Keymap::add_fall_through`],
//! [`Plugin::fall_through_keys`](crate::Plugin::fall_through_keys)), ProseMirror's
//! keymap semantics: where its command does not apply, its key goes on to the
//! page (the web's `keydown` listeners, the browser's default) as an unbound key
//! does. The base keymap declares only Mod-Enter (`exitCode`), so an app's
//! Ctrl/Cmd+Enter still reaches it outside a code block.
//!
//! ## "Mod" / the primary accelerator
//!
//! Bindings are written with `Mod` (e.g. `"Mod-b"`) meaning *the platform's primary
//! accelerator* — Ctrl on Windows/Linux, Cmd on macOS. The core collapses
//! Ctrl/Cmd/Meta/Mod into a single [`Modifiers::primary`] flag; the platform view
//! decides which physical modifier maps to `primary` for the host OS. This keeps
//! one binding table correct on every platform.

use std::collections::{HashMap, HashSet};

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

/// A binding → command-name lookup table, aggregated from all plugins, with the
/// set of bindings that leave their key alone when the command does not apply
/// (the module docs, "Which keys a binding consumes").
#[derive(Debug, Default, Clone)]
pub struct Keymap {
    map: HashMap<KeyBinding, String>,
    fall_through: HashSet<KeyBinding>,
}

impl Keymap {
    /// An empty keymap.
    pub fn new() -> Keymap {
        Keymap::default()
    }

    /// Bind `binding` to the command named `command`, a binding that consumes
    /// its key whether or not the command applies (Tab at the top level, which
    /// sinks no list item, still never moves the focus). Later bindings override
    /// earlier ones for the same key, fall-through included.
    pub fn add(&mut self, binding: KeyBinding, command: impl Into<String>) {
        self.map.insert(binding, command.into());
        self.fall_through.remove(&binding);
    }

    /// Bind `binding` to the command named `command`, a binding that consumes
    /// its key only when the command applies, and otherwise leaves it to the
    /// page (Mod-Enter's `exitCode` outside a code block). Overrides an earlier
    /// binding of the same key.
    pub fn add_fall_through(&mut self, binding: KeyBinding, command: impl Into<String>) {
        self.map.insert(binding, command.into());
        self.fall_through.insert(binding);
    }

    /// Whether `binding` leaves its key alone when its command does not apply
    /// ([`add_fall_through`](Self::add_fall_through)). `false` for an unbound key.
    pub fn falls_through_when_inapplicable(&self, binding: &KeyBinding) -> bool {
        self.fall_through.contains(binding)
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

    #[test]
    fn a_later_binding_replaces_the_fall_through_too() {
        let enter = KeyBinding::parse("Mod-Enter").unwrap();
        let mut km = Keymap::new();
        km.add(enter, "a");
        assert!(
            !km.falls_through_when_inapplicable(&enter),
            "add: consuming"
        );
        km.add_fall_through(enter, "b");
        assert!(km.falls_through_when_inapplicable(&enter));
        assert_eq!(km.command_for(&enter), Some("b"));
        km.add(enter, "c");
        assert!(
            !km.falls_through_when_inapplicable(&enter),
            "a plain rebinding consumes its key again"
        );
        assert!(!km.falls_through_when_inapplicable(&KeyBinding::parse("q").unwrap()));
    }
}
