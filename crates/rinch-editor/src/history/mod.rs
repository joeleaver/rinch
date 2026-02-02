//! History management for undo/redo.

mod local_undo;
mod operations;

pub use local_undo::{LocalUndoStack, UndoItem};
pub use operations::UndoOperation;

use crate::error::EditorError;

/// History manager for undo/redo operations.
#[derive(Debug)]
pub struct History {
    /// Local undo stack
    local: LocalUndoStack,
}

impl History {
    /// Create a new history manager.
    pub fn new() -> Self {
        Self {
            local: LocalUndoStack::new(),
        }
    }

    /// Create with custom capacity.
    pub fn with_capacity(max_size: usize) -> Self {
        Self {
            local: LocalUndoStack::with_capacity(max_size),
        }
    }

    /// Push an undo operation onto the stack.
    pub fn push_undo(&mut self, operation: UndoOperation) {
        let item = UndoItem::new(operation);
        self.local.push(item);
    }

    /// Try to merge typing operations for better grouping.
    /// If the operation can be merged with the last one, returns true.
    /// Otherwise, pushes it as a new operation.
    pub fn merge_typing(&mut self, operation: UndoOperation) {
        if !self.local.merge_last(&operation) {
            self.push_undo(operation);
        }
    }

    /// Undo the last operation.
    /// Returns the inverse operation that was performed, or None if undo stack is empty.
    pub fn undo(&mut self) -> Result<Option<UndoItem>, EditorError> {
        self.local.undo()
    }

    /// Redo the last undone operation.
    /// Returns the operation that was redone, or None if redo stack is empty.
    pub fn redo(&mut self) -> Result<Option<UndoItem>, EditorError> {
        self.local.redo()
    }

    /// Check if undo is available.
    pub fn can_undo(&self) -> bool {
        self.local.can_undo()
    }

    /// Check if redo is available.
    pub fn can_redo(&self) -> bool {
        self.local.can_redo()
    }

    /// Clear all undo/redo history.
    pub fn clear(&mut self) {
        self.local.clear();
    }

    /// Get the number of undo operations available.
    pub fn undo_count(&self) -> usize {
        self.local.undo_count()
    }

    /// Get the number of redo operations available.
    pub fn redo_count(&self) -> usize {
        self.local.redo_count()
    }
}

impl Default for History {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Position;

    #[test]
    fn history_new_is_empty() {
        let history = History::new();
        assert!(!history.can_undo());
        assert!(!history.can_redo());
    }

    #[test]
    fn push_undo_makes_undoable() {
        let mut history = History::new();
        history.push_undo(UndoOperation::InsertText {
            position: Position(0),
            text: "test".into(),
        });
        assert!(history.can_undo());
        assert!(!history.can_redo());
    }

    #[test]
    fn undo_returns_item() {
        let mut history = History::new();
        history.push_undo(UndoOperation::InsertText {
            position: Position(0),
            text: "test".into(),
        });

        let undone = history.undo().unwrap();
        assert!(undone.is_some());
        assert!(!history.can_undo());
        assert!(history.can_redo());
    }

    #[test]
    fn redo_restores_operation() {
        let mut history = History::new();
        history.push_undo(UndoOperation::InsertText {
            position: Position(0),
            text: "test".into(),
        });
        history.undo().unwrap();

        let redone = history.redo().unwrap();
        assert!(redone.is_some());
        assert!(history.can_undo());
        assert!(!history.can_redo());
    }

    #[test]
    fn merge_typing_groups_consecutive_chars() {
        let mut history = History::new();

        // First char - should be pushed
        history.merge_typing(UndoOperation::InsertText {
            position: Position(0),
            text: "h".into(),
        });
        assert_eq!(history.undo_count(), 1);

        // Second char - should be merged
        history.merge_typing(UndoOperation::InsertText {
            position: Position(1),
            text: "i".into(),
        });
        assert_eq!(history.undo_count(), 1);

        // Check merged result
        let item = history.undo().unwrap().unwrap();
        match &item.operation {
            UndoOperation::InsertText { text, .. } => {
                assert_eq!(text, "hi");
            }
            _ => panic!("Expected InsertText"),
        }
    }

    #[test]
    fn merge_typing_does_not_merge_incompatible() {
        let mut history = History::new();

        history.merge_typing(UndoOperation::InsertText {
            position: Position(0),
            text: "a".into(),
        });

        // Different position - should not merge
        history.merge_typing(UndoOperation::InsertText {
            position: Position(10),
            text: "b".into(),
        });

        assert_eq!(history.undo_count(), 2);
    }

    #[test]
    fn clear_removes_all() {
        let mut history = History::new();
        history.push_undo(UndoOperation::InsertText {
            position: Position(0),
            text: "test".into(),
        });
        history.undo().unwrap();

        history.clear();
        assert_eq!(history.undo_count(), 0);
        assert_eq!(history.redo_count(), 0);
    }

    #[test]
    fn with_capacity_limits_size() {
        let mut history = History::with_capacity(2);

        history.push_undo(UndoOperation::InsertText {
            position: Position(0),
            text: "a".into(),
        });
        history.push_undo(UndoOperation::InsertText {
            position: Position(1),
            text: "b".into(),
        });
        history.push_undo(UndoOperation::InsertText {
            position: Position(2),
            text: "c".into(),
        });

        assert_eq!(history.undo_count(), 2);
    }
}

