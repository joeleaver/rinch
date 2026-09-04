/// Minimal set of text editing commands.
#[derive(Clone, Debug, PartialEq)]
pub enum EditCommand {
    // Text modification
    InsertText(String),
    DeleteBackward,     // Backspace
    DeleteForward,      // Delete
    DeleteWordBackward, // Ctrl+Backspace
    DeleteWordForward,  // Ctrl+Delete
    DeleteToLineStart,  // Cmd+Backspace (macOS)
    DeleteToLineEnd,    // Ctrl+K or Cmd+Delete

    // Cursor movement
    MoveLeft,
    MoveRight,
    MoveUp,
    MoveDown,
    MoveWordLeft,    // Ctrl+Left
    MoveWordRight,   // Ctrl+Right
    MoveToLineStart, // Home
    MoveToLineEnd,   // End
    MoveToDocStart,  // Ctrl+Home
    MoveToDocEnd,    // Ctrl+End

    // Selection (same as Move but extends selection)
    SelectLeft,
    SelectRight,
    SelectUp,
    SelectDown,
    SelectWordLeft,
    SelectWordRight,
    SelectToLineStart,
    SelectToLineEnd,
    SelectToDocStart,
    SelectToDocEnd,
    SelectAll,

    // Clipboard
    Cut,
    Copy,
    Paste(String),

    // Undo/Redo
    Undo,
    Redo,

    // Multi-line specific
    InsertNewline,
}

impl EditCommand {
    /// Whether running this command can change the document's text.
    ///
    /// The line a **read-only** field draws: it focuses, moves its caret,
    /// selects and copies like any other field, and refuses only the commands
    /// below. (A **disabled** field refuses everything, and is gated further
    /// up, where the claim itself is decided.)
    ///
    /// The match is exhaustive on purpose — a new variant must be classified
    /// here rather than silently defaulting to "harmless", which for a
    /// text-changing command would be a hole in the read-only guarantee.
    pub fn mutates_text(&self) -> bool {
        match self {
            Self::InsertText(_)
            | Self::DeleteBackward
            | Self::DeleteForward
            | Self::DeleteWordBackward
            | Self::DeleteWordForward
            | Self::DeleteToLineStart
            | Self::DeleteToLineEnd
            | Self::Cut
            | Self::Paste(_)
            | Self::Undo
            | Self::Redo
            | Self::InsertNewline => true,
            Self::MoveLeft
            | Self::MoveRight
            | Self::MoveUp
            | Self::MoveDown
            | Self::MoveWordLeft
            | Self::MoveWordRight
            | Self::MoveToLineStart
            | Self::MoveToLineEnd
            | Self::MoveToDocStart
            | Self::MoveToDocEnd
            | Self::SelectLeft
            | Self::SelectRight
            | Self::SelectUp
            | Self::SelectDown
            | Self::SelectWordLeft
            | Self::SelectWordRight
            | Self::SelectToLineStart
            | Self::SelectToLineEnd
            | Self::SelectToDocStart
            | Self::SelectToDocEnd
            | Self::SelectAll
            | Self::Copy => false,
        }
    }
}

#[cfg(test)]
mod mutates_text_tests {
    use super::EditCommand;

    #[test]
    fn cut_mutates_but_copy_does_not() {
        // The pair a read-only field must separate: Cut removes the selection,
        // Copy only reads it.
        assert!(EditCommand::Cut.mutates_text());
        assert!(!EditCommand::Copy.mutates_text());
    }

    #[test]
    fn history_commands_mutate() {
        // Undo/Redo replay text changes, so a read-only field must refuse them
        // even though the user did not type anything.
        assert!(EditCommand::Undo.mutates_text());
        assert!(EditCommand::Redo.mutates_text());
    }

    #[test]
    fn caret_motion_and_selection_never_mutate() {
        for cmd in [
            EditCommand::MoveLeft,
            EditCommand::MoveToDocEnd,
            EditCommand::SelectAll,
            EditCommand::SelectToLineStart,
        ] {
            assert!(!cmd.mutates_text(), "{cmd:?} must not count as a mutation");
        }
    }
}
