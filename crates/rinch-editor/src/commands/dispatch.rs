//! Command dispatcher for executing document mutations.

use crate::editor::Editor;
use crate::error::EditorError;

/// Trait for commands that can be executed on the editor.
pub trait Command: std::fmt::Debug {
    /// Execute the command.
    fn execute(&self, editor: &mut Editor) -> Result<(), EditorError>;

    /// Get a description of what this command does.
    fn description(&self) -> &str;

    /// Create an inverse command for undo. Returns None if not undoable.
    fn inverse(&self, _editor: &Editor) -> Option<Box<dyn Command>> {
        None
    }
}

/// Dispatcher for executing commands on the editor.
#[derive(Debug)]
pub struct CommandDispatcher {
    /// Command history for undo (stores inverse commands)
    undo_stack: Vec<Box<dyn Command>>,
    /// Redo stack (stores inverse of undone commands)
    redo_stack: Vec<Box<dyn Command>>,
}

impl CommandDispatcher {
    /// Create a new command dispatcher.
    pub fn new() -> Self {
        Self {
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
        }
    }

    /// Execute a command and add its inverse to the undo stack.
    pub fn execute(
        &mut self,
        cmd: Box<dyn Command>,
        editor: &mut Editor,
    ) -> Result<(), EditorError> {
        // Capture inverse before executing (state may change)
        let inverse = cmd.inverse(editor);

        cmd.execute(editor)?;

        if let Some(inv) = inverse {
            self.undo_stack.push(inv);
            // Clear redo stack on new action
            self.redo_stack.clear();
        }

        Ok(())
    }

    /// Undo the last command.
    pub fn undo(&mut self, editor: &mut Editor) -> Result<(), EditorError> {
        if let Some(cmd) = self.undo_stack.pop() {
            let redo_inverse = cmd.inverse(editor);
            cmd.execute(editor)?;
            if let Some(inv) = redo_inverse {
                self.redo_stack.push(inv);
            }
            Ok(())
        } else {
            Err(EditorError::CommandFailed("Nothing to undo".into()))
        }
    }

    /// Redo the last undone command.
    pub fn redo(&mut self, editor: &mut Editor) -> Result<(), EditorError> {
        if let Some(cmd) = self.redo_stack.pop() {
            let undo_inverse = cmd.inverse(editor);
            cmd.execute(editor)?;
            if let Some(inv) = undo_inverse {
                self.undo_stack.push(inv);
            }
            Ok(())
        } else {
            Err(EditorError::CommandFailed("Nothing to redo".into()))
        }
    }

    /// Check if there are commands to undo.
    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    /// Check if there are commands to redo.
    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    /// Clear all history.
    pub fn clear(&mut self) {
        self.undo_stack.clear();
        self.redo_stack.clear();
    }
}

impl Default for CommandDispatcher {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Position;
    use crate::editor::EditorConfig;
    use crate::schema::Schema;

    #[derive(Debug)]
    struct InsertTextCmd {
        pos: usize,
        text: String,
    }

    impl Command for InsertTextCmd {
        fn execute(&self, editor: &mut Editor) -> Result<(), EditorError> {
            editor.doc.insert_text(Position::new(self.pos), &self.text)
        }
        fn description(&self) -> &str {
            "Insert text"
        }
        fn inverse(&self, _editor: &Editor) -> Option<Box<dyn Command>> {
            Some(Box::new(DeleteRangeCmd {
                start: self.pos,
                end: self.pos + self.text.len(),
            }))
        }
    }

    #[derive(Debug)]
    struct DeleteRangeCmd {
        start: usize,
        end: usize,
    }

    impl Command for DeleteRangeCmd {
        fn execute(&self, editor: &mut Editor) -> Result<(), EditorError> {
            editor
                .doc
                .delete_range(crate::document::Range::new(self.start, self.end))
        }
        fn description(&self) -> &str {
            "Delete range"
        }
        fn inverse(&self, editor: &Editor) -> Option<Box<dyn Command>> {
            // Capture text before delete for undo
            let text = editor.doc.to_text();
            let deleted: String = text
                .chars()
                .skip(self.start)
                .take(self.end - self.start)
                .collect();
            Some(Box::new(InsertTextCmd {
                pos: self.start,
                text: deleted,
            }))
        }
    }

    fn test_editor() -> Editor {
        Editor::new(Schema::starter_kit(), EditorConfig::default()).unwrap()
    }

    #[test]
    fn execute_command() {
        let mut editor = test_editor();
        let mut dispatcher = CommandDispatcher::new();
        let cmd = Box::new(InsertTextCmd {
            pos: 0,
            text: "Hello".into(),
        });
        dispatcher.execute(cmd, &mut editor).unwrap();
        assert_eq!(editor.doc.to_text(), "Hello");
    }

    #[test]
    fn undo_command() {
        let mut editor = test_editor();
        let mut dispatcher = CommandDispatcher::new();
        let cmd = Box::new(InsertTextCmd {
            pos: 0,
            text: "Hello".into(),
        });
        dispatcher.execute(cmd, &mut editor).unwrap();
        assert!(dispatcher.can_undo());
        dispatcher.undo(&mut editor).unwrap();
        assert_eq!(editor.doc.to_text(), "");
    }

    #[test]
    fn redo_command() {
        let mut editor = test_editor();
        let mut dispatcher = CommandDispatcher::new();
        let cmd = Box::new(InsertTextCmd {
            pos: 0,
            text: "Hello".into(),
        });
        dispatcher.execute(cmd, &mut editor).unwrap();
        dispatcher.undo(&mut editor).unwrap();
        assert!(dispatcher.can_redo());
        dispatcher.redo(&mut editor).unwrap();
        assert_eq!(editor.doc.to_text(), "Hello");
    }

    #[test]
    fn undo_empty_fails() {
        let mut editor = test_editor();
        let mut dispatcher = CommandDispatcher::new();
        assert!(!dispatcher.can_undo());
        assert!(dispatcher.undo(&mut editor).is_err());
    }

    #[test]
    fn redo_empty_fails() {
        let mut editor = test_editor();
        let mut dispatcher = CommandDispatcher::new();
        assert!(!dispatcher.can_redo());
        assert!(dispatcher.redo(&mut editor).is_err());
    }

    #[test]
    fn new_action_clears_redo() {
        let mut editor = test_editor();
        let mut dispatcher = CommandDispatcher::new();
        dispatcher
            .execute(
                Box::new(InsertTextCmd {
                    pos: 0,
                    text: "A".into(),
                }),
                &mut editor,
            )
            .unwrap();
        dispatcher.undo(&mut editor).unwrap();
        assert!(dispatcher.can_redo());
        dispatcher
            .execute(
                Box::new(InsertTextCmd {
                    pos: 0,
                    text: "B".into(),
                }),
                &mut editor,
            )
            .unwrap();
        assert!(!dispatcher.can_redo());
    }
}
