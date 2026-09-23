//! A key press offered to the app before the editor acts on it — see
//! [`EditorHandle::on_key`](crate::EditorHandle::on_key).
//!
//! The vocabulary is the browser's `KeyboardEvent`, because it is the one both
//! platforms can speak without loss: `key` is the key's name for a named key
//! and the character it types for a printable one, and the modifiers are the
//! four physical ones plus `primary`, the platform's accelerator. The desktop
//! runtime translates winit's key into the same spelling.

/// A key press in a focused editor, offered to
/// [`EditorHandle::on_key`](crate::EditorHandle::on_key) before the editor
/// moves the caret, runs a key binding or inserts text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EditorKey<'a> {
    /// The key, spelled as the browser's
    /// [`KeyboardEvent.key`](https://developer.mozilla.org/en-US/docs/Web/API/UI_Events/Keyboard_event_key_values):
    /// a named key by its name (`"ArrowDown"`, `"Enter"`, `"Escape"`, `"Tab"`,
    /// `"Backspace"`, `"Home"`, `"F2"`), a printable key by the text it types
    /// with the modifiers held (`"["`, `"a"`, `"A"` with Shift, `" "` for the
    /// space bar). A modifier pressed alone is offered too (`"Shift"`,
    /// `"Control"`), as the browser reports it.
    pub key: &'a str,
    /// The platform's accelerator was held: Cmd on macOS, Ctrl elsewhere.
    pub primary: bool,
    /// Control was held.
    pub ctrl: bool,
    /// Meta (Cmd on macOS, the Windows/Super key elsewhere) was held.
    pub meta: bool,
    /// Shift was held.
    pub shift: bool,
    /// Alt (Option on macOS) was held.
    pub alt: bool,
    /// The press is the operating system's auto-repeat of a held key.
    pub repeat: bool,
}
