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

    // Block structure
    Indent,  // Tab in list item
    Outdent, // Shift+Tab in list item

    // Inline formatting
    ToggleBold,
    ToggleItalic,
    ToggleUnderline,
    ToggleStrikethrough,
    ToggleCode,

    // Block type changes
    SetBlockType(String), // "paragraph", "heading1", etc.
    ToggleBulletList,
    ToggleOrderedList,
    InsertHorizontalRule,
    ToggleBlockquote,
}
