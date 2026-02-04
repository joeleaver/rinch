//! Keyboard shortcut handling.

use crate::editor::Editor;
use crate::error::EditorError;
use std::collections::HashMap;

/// Parsed key combination.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ParsedKey {
    ctrl: bool,
    shift: bool,
    alt: bool,
    meta: bool,
    key: String,
}

/// Parse a key string like "Mod-b", "Ctrl+Shift+L", "Alt-x" into a normalized form.
fn parse_key_string(s: &str) -> ParsedKey {
    let mut ctrl = false;
    let mut shift = false;
    let mut alt = false;
    let mut meta = false;
    let mut key = String::new();

    // Split by '-' or '+'
    let parts: Vec<&str> = s.split(['-', '+']).collect();

    for part in &parts {
        let lower = part.to_lowercase();
        match lower.as_str() {
            "mod" | "ctrl" | "control" => ctrl = true,
            "shift" => shift = true,
            "alt" => alt = true,
            "meta" | "cmd" | "command" | "super" => meta = true,
            _ => key = lower,
        }
    }

    // "Mod" means Ctrl on Windows/Linux, Cmd on Mac.
    // We normalize "Mod" to "ctrl" for matching purposes since
    // key events will be pre-normalized by the platform layer.
    // The original string with "Mod" sets ctrl=true above.

    ParsedKey {
        ctrl,
        shift,
        alt,
        meta,
        key,
    }
}

/// Normalize a key event string to a canonical form for matching.
fn normalize_key(s: &str) -> String {
    let parsed = parse_key_string(s);
    let mut parts = Vec::new();
    if parsed.ctrl {
        parts.push("ctrl");
    }
    if parsed.shift {
        parts.push("shift");
    }
    if parsed.alt {
        parts.push("alt");
    }
    if parsed.meta {
        parts.push("meta");
    }
    parts.push(&parsed.key);
    parts.join("+")
}

/// A keyboard shortcut binding.
#[derive(Debug, Clone)]
pub struct KeyboardShortcut {
    /// Key combination (e.g., "Ctrl+B", "Mod-b")
    pub key: String,
    /// Normalized key for matching
    normalized: String,
    /// Description of what this shortcut does
    pub description: String,
}

impl KeyboardShortcut {
    /// Create a new keyboard shortcut.
    pub fn new(key: impl Into<String>, description: impl Into<String>) -> Self {
        let key = key.into();
        let normalized = normalize_key(&key);
        Self {
            key,
            normalized,
            description: description.into(),
        }
    }

    /// Check if a key event matches this shortcut.
    /// The key_event should be in a format like "Ctrl+b", "Shift+Alt+x", etc.
    pub fn matches(&self, key_event: &str) -> bool {
        normalize_key(key_event) == self.normalized
    }
}

/// Registry for keyboard shortcuts.
#[derive(Debug, Default)]
pub struct ShortcutRegistry {
    /// Map of normalized key combinations to command names
    shortcuts: HashMap<String, String>,
}

impl ShortcutRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a keyboard shortcut.
    pub fn register(&mut self, shortcut: KeyboardShortcut, command: impl Into<String>) {
        self.shortcuts.insert(shortcut.normalized, command.into());
    }

    /// Look up a command by key event. Returns the command name if found.
    pub fn lookup(&self, key_event: &str) -> Option<&str> {
        let normalized = normalize_key(key_event);
        self.shortcuts.get(&normalized).map(|s| s.as_str())
    }

    /// Handle a key event: look up and execute the associated command via the extension registry.
    /// Returns Ok(true) if a command was found and executed.
    pub fn handle_key(&self, editor: &mut Editor, key_event: &str) -> Result<bool, EditorError> {
        let normalized = normalize_key(key_event);
        if let Some(command_name) = self.shortcuts.get(&normalized) {
            if let Some(handler) = editor.extensions.get_command(command_name) {
                handler(editor)?;
                return Ok(true);
            }
            return Err(EditorError::UnknownCommand(command_name.clone()));
        }
        Ok(false)
    }

    /// Get number of registered shortcuts.
    pub fn len(&self) -> usize {
        self.shortcuts.len()
    }

    /// Check if empty.
    pub fn is_empty(&self) -> bool {
        self.shortcuts.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_mod_b() {
        let p = parse_key_string("Mod-b");
        assert!(p.ctrl);
        assert_eq!(p.key, "b");
    }

    #[test]
    fn parse_ctrl_shift_l() {
        let p = parse_key_string("Ctrl+Shift+L");
        assert!(p.ctrl);
        assert!(p.shift);
        assert_eq!(p.key, "l");
    }

    #[test]
    fn parse_alt_x() {
        let p = parse_key_string("Alt-x");
        assert!(p.alt);
        assert_eq!(p.key, "x");
    }

    #[test]
    fn shortcut_matches_same_format() {
        let s = KeyboardShortcut::new("Mod-b", "Bold");
        assert!(s.matches("Ctrl-b"));
        assert!(s.matches("Ctrl+b"));
        assert!(s.matches("Mod-b"));
    }

    #[test]
    fn shortcut_does_not_match_different_key() {
        let s = KeyboardShortcut::new("Mod-b", "Bold");
        assert!(!s.matches("Ctrl-i"));
        assert!(!s.matches("Alt-b"));
    }

    #[test]
    fn shortcut_matches_case_insensitive() {
        let s = KeyboardShortcut::new("Ctrl+Shift+L", "List");
        assert!(s.matches("ctrl+shift+l"));
        assert!(s.matches("Ctrl+Shift+L"));
    }

    #[test]
    fn registry_lookup() {
        let mut reg = ShortcutRegistry::new();
        reg.register(KeyboardShortcut::new("Mod-b", "Bold"), "toggle_bold");
        assert_eq!(reg.lookup("Ctrl-b"), Some("toggle_bold"));
        assert_eq!(reg.lookup("Alt-b"), None);
    }

    #[test]
    fn registry_len() {
        let mut reg = ShortcutRegistry::new();
        assert!(reg.is_empty());
        reg.register(KeyboardShortcut::new("Mod-b", "Bold"), "toggle_bold");
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn normalize_key_consistent() {
        assert_eq!(normalize_key("Mod-b"), normalize_key("Ctrl+b"));
        assert_eq!(normalize_key("Ctrl+Shift+L"), normalize_key("Shift-Ctrl-l"));
    }
}
