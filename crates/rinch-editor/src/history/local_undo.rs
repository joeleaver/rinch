//! Local undo stack for single-user undo/redo.

use crate::error::EditorError;

use super::operations::UndoOperation;

/// Local undo stack that works independently of collaborative editing.
#[derive(Debug)]
pub struct LocalUndoStack {
    /// Stack of undo items
    undo_stack: Vec<UndoItem>,
    /// Stack of redo items
    redo_stack: Vec<UndoItem>,
    /// Maximum stack size
    max_size: usize,
}

impl LocalUndoStack {
    /// Create a new undo stack with default max size.
    pub fn new() -> Self {
        Self::with_capacity(100)
    }

    /// Create a new undo stack with specified max size.
    pub fn with_capacity(max_size: usize) -> Self {
        Self {
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            max_size,
        }
    }

    /// Add an item to the undo stack.
    pub fn push(&mut self, item: UndoItem) {
        // Clear redo stack when new operation is performed
        self.redo_stack.clear();

        // Enforce max size by removing oldest items
        if self.undo_stack.len() >= self.max_size {
            self.undo_stack.remove(0);
        }

        self.undo_stack.push(item);
    }

    /// Undo the last operation.
    pub fn undo(&mut self) -> Result<Option<UndoItem>, EditorError> {
        if let Some(item) = self.undo_stack.pop() {
            self.redo_stack.push(item.clone());
            Ok(Some(item))
        } else {
            Ok(None)
        }
    }

    /// Redo the last undone operation.
    pub fn redo(&mut self) -> Result<Option<UndoItem>, EditorError> {
        if let Some(item) = self.redo_stack.pop() {
            self.undo_stack.push(item.clone());
            Ok(Some(item))
        } else {
            Ok(None)
        }
    }

    /// Check if undo is available.
    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    /// Check if redo is available.
    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    /// Get the number of undo operations available.
    pub fn undo_count(&self) -> usize {
        self.undo_stack.len()
    }

    /// Get the number of redo operations available.
    pub fn redo_count(&self) -> usize {
        self.redo_stack.len()
    }

    /// Merge the last operation with a new one if possible.
    /// Used for grouping consecutive typing into a single undo step.
    /// Returns true if merge was successful.
    pub fn merge_last(&mut self, new_op: &UndoOperation) -> bool {
        if let Some(last_item) = self.undo_stack.last_mut() {
            if last_item.operation.can_merge(new_op) {
                last_item.operation.merge(new_op);
                return true;
            }
        }
        false
    }

    /// Clear the undo and redo stacks.
    pub fn clear(&mut self) {
        self.undo_stack.clear();
        self.redo_stack.clear();
    }
}

impl Default for LocalUndoStack {
    fn default() -> Self {
        Self::new()
    }
}

/// An item in the undo stack.
#[derive(Debug, Clone)]
pub struct UndoItem {
    /// The operation to undo
    pub operation: UndoOperation,
}

impl UndoItem {
    /// Create a new undo item.
    pub fn new(operation: UndoOperation) -> Self {
        Self { operation }
    }

    /// Get the inverse operation that undoes this item.
    pub fn inverse(&self) -> UndoItem {
        UndoItem {
            operation: self.operation.inverse(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Position, Range};
    use std::collections::HashMap;

    #[test]
    fn new_stack_is_empty() {
        let stack = LocalUndoStack::new();
        assert_eq!(stack.undo_count(), 0);
        assert_eq!(stack.redo_count(), 0);
        assert!(!stack.can_undo());
        assert!(!stack.can_redo());
    }

    #[test]
    fn push_adds_to_undo_stack() {
        let mut stack = LocalUndoStack::new();
        let item = UndoItem::new(UndoOperation::InsertText {
            position: Position(0),
            text: "Hello".into(),
        });
        stack.push(item);

        assert_eq!(stack.undo_count(), 1);
        assert!(stack.can_undo());
        assert!(!stack.can_redo());
    }

    #[test]
    fn push_clears_redo_stack() {
        let mut stack = LocalUndoStack::new();
        let item1 = UndoItem::new(UndoOperation::InsertText {
            position: Position(0),
            text: "a".into(),
        });
        let item2 = UndoItem::new(UndoOperation::InsertText {
            position: Position(1),
            text: "b".into(),
        });

        stack.push(item1);
        stack.undo().unwrap();
        assert_eq!(stack.redo_count(), 1);

        // Push new item should clear redo
        stack.push(item2);
        assert_eq!(stack.redo_count(), 0);
    }

    #[test]
    fn undo_pops_from_undo_pushes_to_redo() {
        let mut stack = LocalUndoStack::new();
        let item = UndoItem::new(UndoOperation::InsertText {
            position: Position(0),
            text: "Hello".into(),
        });
        stack.push(item.clone());

        let undone = stack.undo().unwrap();
        assert!(undone.is_some());
        assert_eq!(stack.undo_count(), 0);
        assert_eq!(stack.redo_count(), 1);
        assert!(stack.can_redo());
    }

    #[test]
    fn undo_empty_stack_returns_none() {
        let mut stack = LocalUndoStack::new();
        let result = stack.undo().unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn redo_pops_from_redo_pushes_to_undo() {
        let mut stack = LocalUndoStack::new();
        let item = UndoItem::new(UndoOperation::InsertText {
            position: Position(0),
            text: "Hello".into(),
        });
        stack.push(item);
        stack.undo().unwrap();

        let redone = stack.redo().unwrap();
        assert!(redone.is_some());
        assert_eq!(stack.undo_count(), 1);
        assert_eq!(stack.redo_count(), 0);
        assert!(stack.can_undo());
        assert!(!stack.can_redo());
    }

    #[test]
    fn redo_empty_stack_returns_none() {
        let mut stack = LocalUndoStack::new();
        let result = stack.redo().unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn undo_redo_cycle() {
        let mut stack = LocalUndoStack::new();
        let item = UndoItem::new(UndoOperation::InsertText {
            position: Position(5),
            text: "World".into(),
        });
        stack.push(item);

        // Undo
        let undone = stack.undo().unwrap().unwrap();
        assert_eq!(stack.undo_count(), 0);
        assert_eq!(stack.redo_count(), 1);

        // Redo
        let redone = stack.redo().unwrap().unwrap();
        assert_eq!(stack.undo_count(), 1);
        assert_eq!(stack.redo_count(), 0);

        // Should have same operation
        match (&undone.operation, &redone.operation) {
            (
                UndoOperation::InsertText {
                    position: p1,
                    text: t1,
                },
                UndoOperation::InsertText {
                    position: p2,
                    text: t2,
                },
            ) => {
                assert_eq!(p1, p2);
                assert_eq!(t1, t2);
            }
            _ => panic!("Operations don't match"),
        }
    }

    #[test]
    fn max_size_enforcement() {
        let mut stack = LocalUndoStack::with_capacity(3);
        for i in 0..5 {
            let item = UndoItem::new(UndoOperation::InsertText {
                position: Position(i),
                text: i.to_string(),
            });
            stack.push(item);
        }

        // Should only have last 3 items
        assert_eq!(stack.undo_count(), 3);

        // Check we have items 2, 3, 4 (oldest 0, 1 were removed)
        let item4 = stack.undo().unwrap().unwrap();
        let item3 = stack.undo().unwrap().unwrap();
        let item2 = stack.undo().unwrap().unwrap();

        match (
            &item2.operation,
            &item3.operation,
            &item4.operation,
        ) {
            (
                UndoOperation::InsertText { text: t2, .. },
                UndoOperation::InsertText { text: t3, .. },
                UndoOperation::InsertText { text: t4, .. },
            ) => {
                assert_eq!(t2, "2");
                assert_eq!(t3, "3");
                assert_eq!(t4, "4");
            }
            _ => panic!("Wrong operations"),
        }
    }

    #[test]
    fn clear_empties_both_stacks() {
        let mut stack = LocalUndoStack::new();
        stack.push(UndoItem::new(UndoOperation::InsertText {
            position: Position(0),
            text: "a".into(),
        }));
        stack.undo().unwrap();

        assert_eq!(stack.undo_count(), 0);
        assert_eq!(stack.redo_count(), 1);

        stack.clear();
        assert_eq!(stack.undo_count(), 0);
        assert_eq!(stack.redo_count(), 0);
    }

    #[test]
    fn merge_last_consecutive_typing() {
        let mut stack = LocalUndoStack::new();

        // Push first character
        stack.push(UndoItem::new(UndoOperation::InsertText {
            position: Position(0),
            text: "h".into(),
        }));

        // Merge second character
        let merged = stack.merge_last(&UndoOperation::InsertText {
            position: Position(1),
            text: "e".into(),
        });
        assert!(merged);
        assert_eq!(stack.undo_count(), 1);

        // Check merged text
        let item = stack.undo().unwrap().unwrap();
        match &item.operation {
            UndoOperation::InsertText { text, .. } => {
                assert_eq!(text, "he");
            }
            _ => panic!("Expected InsertText"),
        }
    }

    #[test]
    fn merge_last_fails_for_non_consecutive() {
        let mut stack = LocalUndoStack::new();

        stack.push(UndoItem::new(UndoOperation::InsertText {
            position: Position(0),
            text: "a".into(),
        }));

        let merged = stack.merge_last(&UndoOperation::InsertText {
            position: Position(10),
            text: "b".into(),
        });
        assert!(!merged);
        assert_eq!(stack.undo_count(), 1);
    }

    #[test]
    fn merge_last_fails_for_different_operation_types() {
        let mut stack = LocalUndoStack::new();

        stack.push(UndoItem::new(UndoOperation::InsertText {
            position: Position(0),
            text: "a".into(),
        }));

        let merged = stack.merge_last(&UndoOperation::DeleteText {
            range: Range::new(0usize, 1usize),
            deleted_text: "a".into(),
        });
        assert!(!merged);
    }

    #[test]
    fn merge_last_on_empty_stack() {
        let mut stack = LocalUndoStack::new();

        let merged = stack.merge_last(&UndoOperation::InsertText {
            position: Position(0),
            text: "a".into(),
        });
        assert!(!merged);
    }

    #[test]
    fn multiple_operations() {
        let mut stack = LocalUndoStack::new();

        // Insert text
        stack.push(UndoItem::new(UndoOperation::InsertText {
            position: Position(0),
            text: "Hello".into(),
        }));

        // Add mark
        stack.push(UndoItem::new(UndoOperation::AddMark {
            range: Range::new(0usize, 5usize),
            mark_type: "bold".into(),
            attrs: HashMap::new(),
        }));

        // Delete text
        stack.push(UndoItem::new(UndoOperation::DeleteText {
            range: Range::new(0usize, 2usize),
            deleted_text: "He".into(),
        }));

        assert_eq!(stack.undo_count(), 3);

        // Undo delete
        let op1 = stack.undo().unwrap().unwrap();
        assert!(matches!(op1.operation, UndoOperation::DeleteText { .. }));

        // Undo add mark
        let op2 = stack.undo().unwrap().unwrap();
        assert!(matches!(op2.operation, UndoOperation::AddMark { .. }));

        // Undo insert
        let op3 = stack.undo().unwrap().unwrap();
        assert!(matches!(op3.operation, UndoOperation::InsertText { .. }));

        assert_eq!(stack.undo_count(), 0);
        assert_eq!(stack.redo_count(), 3);
    }

    #[test]
    fn undo_item_inverse() {
        let item = UndoItem::new(UndoOperation::InsertText {
            position: Position(5),
            text: "test".into(),
        });
        let inv = item.inverse();

        match &inv.operation {
            UndoOperation::DeleteText { range, deleted_text } => {
                assert_eq!(range.start, Position(5));
                assert_eq!(range.end, Position(9));
                assert_eq!(deleted_text, "test");
            }
            _ => panic!("Expected DeleteText"),
        }
    }
}

