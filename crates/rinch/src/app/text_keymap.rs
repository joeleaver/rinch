//! The caret keys of a text field, per platform (issue #951).
//!
//! What an arrow, Home, End, Backspace or Delete does in an `<input>`, a
//! `<textarea>` and the rich-text editor depends on the platform, and the
//! answer is a pure function of the key, the modifiers held and the platform
//! ([`text_key_action`]) — so the macOS answer is pinned by tests that run on
//! Linux, by asking for [`TextKeymap::MacOs`].
//!
//! **Windows and Linux** (Chrome 153 on Linux, and the keymap rinch has always
//! had): Ctrl+Left / Ctrl+Right move by word; Home / End go to the visual
//! line's edges and Ctrl+Home / Ctrl+End to the value's; Up / Down move a line;
//! Ctrl+Backspace / Ctrl+Delete delete a word.
//!
//! **macOS** (Cocoa's `NSStandardKeyBindingResponding`, which Chrome and Safari
//! follow in their text controls): Option+Left / Option+Right move by word;
//! Cmd+Left / Cmd+Right go to the visual line's edges and Cmd+Up / Cmd+Down to
//! the value's; Option+Backspace / Option+Delete delete a word — the chord the
//! rich editor's keymap binds there (`word_delete_bindings` in
//! `rinch-editor-core`). Home and End scroll the field without moving the
//! caret; rinch does not scroll a field's text, so they do nothing, and
//! Shift+Home / Shift+End extend the selection to the value's ends.
//!
//! Shift extends in every case: the head moves and the anchor stays.

use rinch_platform::{KeyCode, Modifiers};

/// Which platform's text-editing keys apply. [`TextKeymap::host`] is the one
/// the running build uses; tests set the other on a `RinchApp` to drive it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextKeymap {
    /// Windows, Linux, and every other platform rinch runs on.
    Standard,
    /// macOS.
    MacOs,
}

impl TextKeymap {
    /// The keymap of the platform this build targets.
    pub const fn host() -> Self {
        if cfg!(target_os = "macos") {
            Self::MacOs
        } else {
            Self::Standard
        }
    }
}

/// Where a caret key sends the head.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TextMotion {
    CharLeft,
    CharRight,
    WordLeft,
    WordRight,
    /// The start of the caret's visual line (of the value, in an `<input>`).
    LineStart,
    /// The end of the caret's visual line (of the value, in an `<input>`).
    LineEnd,
    LineUp,
    LineDown,
    DocStart,
    DocEnd,
}

/// What a text field does with a key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TextKeyAction {
    /// Move the head; `extend` (Shift) keeps the anchor.
    Move { motion: TextMotion, extend: bool },
    /// Backspace: a character, or with the word chord a word.
    DeleteBackward { word: bool },
    /// Delete: a character, or with the word chord a word.
    DeleteForward { word: bool },
    /// The key is the field's and changes nothing in it (macOS Home / End,
    /// which scroll a field rinch does not scroll).
    Inert,
}

/// What `key` with `modifiers` does in a text field under `keymap`, or `None`
/// for a key that is not a caret or delete key.
pub(crate) fn text_key_action(
    keymap: TextKeymap,
    key: KeyCode,
    modifiers: Modifiers,
) -> Option<TextKeyAction> {
    use TextKeyAction::*;
    use TextMotion::*;
    let extend = modifiers.shift;
    let mv = |motion| Some(Move { motion, extend });
    match keymap {
        TextKeymap::Standard => {
            let ctrl = modifiers.ctrl;
            match key {
                KeyCode::ArrowLeft => mv(if ctrl { WordLeft } else { CharLeft }),
                KeyCode::ArrowRight => mv(if ctrl { WordRight } else { CharRight }),
                KeyCode::ArrowUp => mv(LineUp),
                KeyCode::ArrowDown => mv(LineDown),
                KeyCode::Home => mv(if ctrl { DocStart } else { LineStart }),
                KeyCode::End => mv(if ctrl { DocEnd } else { LineEnd }),
                KeyCode::Backspace => Some(DeleteBackward { word: ctrl }),
                KeyCode::Delete => Some(DeleteForward { word: ctrl }),
                _ => None,
            }
        }
        TextKeymap::MacOs => {
            let cmd = modifiers.meta;
            let option = modifiers.alt;
            match key {
                KeyCode::ArrowLeft if cmd => mv(LineStart),
                KeyCode::ArrowRight if cmd => mv(LineEnd),
                KeyCode::ArrowLeft => mv(if option { WordLeft } else { CharLeft }),
                KeyCode::ArrowRight => mv(if option { WordRight } else { CharRight }),
                KeyCode::ArrowUp => mv(if cmd { DocStart } else { LineUp }),
                KeyCode::ArrowDown => mv(if cmd { DocEnd } else { LineDown }),
                KeyCode::Home if extend => mv(DocStart),
                KeyCode::End if extend => mv(DocEnd),
                KeyCode::Home | KeyCode::End => Some(Inert),
                KeyCode::Backspace => Some(DeleteBackward { word: option }),
                KeyCode::Delete => Some(DeleteForward { word: option }),
                _ => None,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use TextKeyAction::*;
    use TextMotion::*;

    fn mods(ctrl: bool, alt: bool, meta: bool, shift: bool) -> Modifiers {
        Modifiers {
            shift,
            ctrl,
            alt,
            meta,
        }
    }
    const NONE: Modifiers = Modifiers {
        shift: false,
        ctrl: false,
        alt: false,
        meta: false,
    };
    fn mv(motion: TextMotion, extend: bool) -> Option<TextKeyAction> {
        Some(Move { motion, extend })
    }

    #[test]
    fn the_host_keymap_follows_the_target() {
        assert_eq!(
            TextKeymap::host() == TextKeymap::MacOs,
            cfg!(target_os = "macos")
        );
    }

    /// Every chord the issue names, on macOS.
    #[test]
    fn macos_chords() {
        let k = |key, m| text_key_action(TextKeymap::MacOs, key, m);
        let cmd = mods(false, false, true, false);
        let cmd_shift = mods(false, false, true, true);
        let opt = mods(false, true, false, false);
        let opt_shift = mods(false, true, false, true);
        let shift = mods(false, false, false, true);
        let ctrl = mods(true, false, false, false);

        assert_eq!(k(KeyCode::ArrowLeft, cmd), mv(LineStart, false));
        assert_eq!(k(KeyCode::ArrowRight, cmd), mv(LineEnd, false));
        assert_eq!(k(KeyCode::ArrowLeft, cmd_shift), mv(LineStart, true));
        assert_eq!(k(KeyCode::ArrowRight, cmd_shift), mv(LineEnd, true));
        assert_eq!(k(KeyCode::ArrowUp, cmd), mv(DocStart, false));
        assert_eq!(k(KeyCode::ArrowDown, cmd), mv(DocEnd, false));
        assert_eq!(k(KeyCode::ArrowDown, cmd_shift), mv(DocEnd, true));
        assert_eq!(k(KeyCode::ArrowLeft, opt), mv(WordLeft, false));
        assert_eq!(k(KeyCode::ArrowRight, opt_shift), mv(WordRight, true));
        assert_eq!(k(KeyCode::ArrowLeft, NONE), mv(CharLeft, false));
        assert_eq!(k(KeyCode::ArrowUp, NONE), mv(LineUp, false));
        // Ctrl is not the word modifier on macOS.
        assert_eq!(k(KeyCode::ArrowLeft, ctrl), mv(CharLeft, false));
        // Home / End do not move the caret; with Shift they extend to the ends.
        assert_eq!(k(KeyCode::Home, NONE), Some(Inert));
        assert_eq!(k(KeyCode::End, NONE), Some(Inert));
        assert_eq!(k(KeyCode::Home, cmd), Some(Inert));
        assert_eq!(k(KeyCode::Home, shift), mv(DocStart, true));
        assert_eq!(k(KeyCode::End, shift), mv(DocEnd, true));
        // The rich editor's word-delete chord.
        assert_eq!(
            k(KeyCode::Backspace, opt),
            Some(DeleteBackward { word: true })
        );
        assert_eq!(k(KeyCode::Delete, opt), Some(DeleteForward { word: true }));
        assert_eq!(
            k(KeyCode::Backspace, ctrl),
            Some(DeleteBackward { word: false })
        );
        assert_eq!(k(KeyCode::KeyA, cmd), None);
    }

    /// Windows and Linux: what rinch has always done, plus the word deletes.
    #[test]
    fn standard_chords() {
        let k = |key, m| text_key_action(TextKeymap::Standard, key, m);
        let ctrl = mods(true, false, false, false);
        let ctrl_shift = mods(true, false, false, true);
        let alt = mods(false, true, false, false);
        let meta = mods(false, false, true, false);

        assert_eq!(k(KeyCode::ArrowLeft, ctrl), mv(WordLeft, false));
        assert_eq!(k(KeyCode::ArrowRight, ctrl_shift), mv(WordRight, true));
        assert_eq!(k(KeyCode::ArrowLeft, alt), mv(CharLeft, false));
        assert_eq!(k(KeyCode::ArrowLeft, meta), mv(CharLeft, false));
        assert_eq!(k(KeyCode::ArrowUp, ctrl), mv(LineUp, false));
        assert_eq!(k(KeyCode::Home, NONE), mv(LineStart, false));
        assert_eq!(k(KeyCode::End, ctrl_shift), mv(DocEnd, true));
        assert_eq!(k(KeyCode::Home, ctrl), mv(DocStart, false));
        assert_eq!(
            k(KeyCode::Backspace, ctrl),
            Some(DeleteBackward { word: true })
        );
        assert_eq!(
            k(KeyCode::Backspace, alt),
            Some(DeleteBackward { word: false })
        );
        assert_eq!(k(KeyCode::Delete, ctrl), Some(DeleteForward { word: true }));
        assert_eq!(k(KeyCode::Enter, NONE), None);
    }
}
