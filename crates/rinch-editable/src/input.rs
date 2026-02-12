use crate::EditCommand;

/// Keyboard modifiers state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub meta: bool, // Cmd on macOS, Win on Windows
}

impl Modifiers {
    pub fn ctrl() -> Self {
        Self {
            ctrl: true,
            ..Default::default()
        }
    }

    pub fn shift() -> Self {
        Self {
            shift: true,
            ..Default::default()
        }
    }

    pub fn ctrl_shift() -> Self {
        Self {
            ctrl: true,
            shift: true,
            ..Default::default()
        }
    }

    pub fn none() -> Self {
        Self::default()
    }

    /// Check if this is a "word" modifier (Ctrl on Windows/Linux, Alt on macOS).
    pub fn is_word_modifier(&self, is_macos: bool) -> bool {
        if is_macos {
            self.alt
        } else {
            self.ctrl
        }
    }

    /// Check if this is a "line" modifier (typically Home/End behavior).
    pub fn is_line_modifier(&self, is_macos: bool) -> bool {
        if is_macos {
            self.meta
        } else {
            false // On Windows/Linux, Home/End are unmodified
        }
    }
}

/// Named keys that map to edit commands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    Backspace,
    Delete,
    Enter,
    Tab,
    Escape,
    // Letters for shortcuts
    A,
    C,
    V,
    X,
    Z,
    Y,
}

/// Maps keyboard input to edit commands.
#[derive(Debug, Default)]
pub struct InputHandler {
    /// Whether we're on macOS (affects modifier keys).
    pub is_macos: bool,
    /// Whether multi-line editing is allowed.
    pub multiline: bool,
}

impl InputHandler {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_macos(mut self, is_macos: bool) -> Self {
        self.is_macos = is_macos;
        self
    }

    pub fn with_multiline(mut self, multiline: bool) -> Self {
        self.multiline = multiline;
        self
    }

    /// Map a key press to an edit command.
    pub fn map_key(&self, key: Key, modifiers: Modifiers) -> Option<EditCommand> {
        // Platform-specific primary modifier
        let primary_mod = if self.is_macos {
            modifiers.meta
        } else {
            modifiers.ctrl
        };

        // Word-level modifier (Ctrl on Win/Linux, Alt on macOS)
        let word_mod = modifiers.is_word_modifier(self.is_macos);

        match key {
            // Navigation
            Key::Left => {
                if word_mod && modifiers.shift {
                    Some(EditCommand::SelectWordLeft)
                } else if word_mod {
                    Some(EditCommand::MoveWordLeft)
                } else if modifiers.shift {
                    Some(EditCommand::SelectLeft)
                } else {
                    Some(EditCommand::MoveLeft)
                }
            }
            Key::Right => {
                if word_mod && modifiers.shift {
                    Some(EditCommand::SelectWordRight)
                } else if word_mod {
                    Some(EditCommand::MoveWordRight)
                } else if modifiers.shift {
                    Some(EditCommand::SelectRight)
                } else {
                    Some(EditCommand::MoveRight)
                }
            }
            Key::Up => {
                if modifiers.shift {
                    Some(EditCommand::SelectUp)
                } else {
                    Some(EditCommand::MoveUp)
                }
            }
            Key::Down => {
                if modifiers.shift {
                    Some(EditCommand::SelectDown)
                } else {
                    Some(EditCommand::MoveDown)
                }
            }
            Key::Home => {
                if primary_mod && modifiers.shift {
                    Some(EditCommand::SelectToDocStart)
                } else if primary_mod {
                    Some(EditCommand::MoveToDocStart)
                } else if modifiers.shift {
                    Some(EditCommand::SelectToLineStart)
                } else {
                    Some(EditCommand::MoveToLineStart)
                }
            }
            Key::End => {
                if primary_mod && modifiers.shift {
                    Some(EditCommand::SelectToDocEnd)
                } else if primary_mod {
                    Some(EditCommand::MoveToDocEnd)
                } else if modifiers.shift {
                    Some(EditCommand::SelectToLineEnd)
                } else {
                    Some(EditCommand::MoveToLineEnd)
                }
            }

            // Deletion
            Key::Backspace => {
                if primary_mod {
                    Some(EditCommand::DeleteToLineStart)
                } else if word_mod {
                    Some(EditCommand::DeleteWordBackward)
                } else {
                    Some(EditCommand::DeleteBackward)
                }
            }
            Key::Delete => {
                if primary_mod {
                    Some(EditCommand::DeleteToLineEnd)
                } else if word_mod {
                    Some(EditCommand::DeleteWordForward)
                } else {
                    Some(EditCommand::DeleteForward)
                }
            }

            // Enter
            Key::Enter => {
                if self.multiline {
                    Some(EditCommand::InsertNewline)
                } else {
                    None // Single-line: don't handle Enter
                }
            }

            // Shortcuts
            Key::A if primary_mod => Some(EditCommand::SelectAll),
            Key::C if primary_mod => Some(EditCommand::Copy),
            Key::X if primary_mod => Some(EditCommand::Cut),
            Key::Z if primary_mod && modifiers.shift => Some(EditCommand::Redo),
            Key::Z if primary_mod => Some(EditCommand::Undo),
            Key::Y if primary_mod => Some(EditCommand::Redo),

            // Indent / Outdent
            Key::Tab => {
                if modifiers.shift {
                    Some(EditCommand::Outdent)
                } else {
                    Some(EditCommand::Indent)
                }
            }

            // Not handled
            _ => None,
        }
    }

    /// Map text input (printable characters) to an insert command.
    pub fn map_text(&self, text: &str) -> Option<EditCommand> {
        if text.is_empty() {
            return None;
        }

        // Filter out control characters except for allowed ones
        let filtered: String = text
            .chars()
            .filter(|c| !c.is_control() || (self.multiline && *c == '\n') || *c == '\t')
            .collect();

        if filtered.is_empty() {
            None
        } else {
            Some(EditCommand::InsertText(filtered))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_navigation() {
        let handler = InputHandler::new();

        assert_eq!(
            handler.map_key(Key::Left, Modifiers::none()),
            Some(EditCommand::MoveLeft)
        );
        assert_eq!(
            handler.map_key(Key::Right, Modifiers::shift()),
            Some(EditCommand::SelectRight)
        );
    }

    #[test]
    fn test_word_navigation_windows() {
        let handler = InputHandler::new().with_macos(false);

        assert_eq!(
            handler.map_key(Key::Left, Modifiers::ctrl()),
            Some(EditCommand::MoveWordLeft)
        );
        assert_eq!(
            handler.map_key(Key::Right, Modifiers::ctrl_shift()),
            Some(EditCommand::SelectWordRight)
        );
    }

    #[test]
    fn test_shortcuts() {
        let handler = InputHandler::new().with_macos(false);

        assert_eq!(
            handler.map_key(Key::A, Modifiers::ctrl()),
            Some(EditCommand::SelectAll)
        );
        assert_eq!(
            handler.map_key(Key::Z, Modifiers::ctrl()),
            Some(EditCommand::Undo)
        );
        assert_eq!(
            handler.map_key(Key::Z, Modifiers::ctrl_shift()),
            Some(EditCommand::Redo)
        );
    }

    #[test]
    fn test_multiline_enter() {
        let single = InputHandler::new().with_multiline(false);
        let multi = InputHandler::new().with_multiline(true);

        assert_eq!(single.map_key(Key::Enter, Modifiers::none()), None);
        assert_eq!(
            multi.map_key(Key::Enter, Modifiers::none()),
            Some(EditCommand::InsertNewline)
        );
    }

    #[test]
    fn test_text_input() {
        let handler = InputHandler::new();

        assert_eq!(
            handler.map_text("hello"),
            Some(EditCommand::InsertText("hello".to_string()))
        );
        assert_eq!(handler.map_text(""), None);
    }
}
