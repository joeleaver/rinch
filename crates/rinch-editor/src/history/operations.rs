//! Undo operation types and their inverses.

use std::collections::HashMap;

use crate::document::{Position, Range};

/// An operation that can be undone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UndoOperation {
    /// Insert text at a position.
    InsertText { position: Position, text: String },

    /// Delete text in a range.
    DeleteText { range: Range, deleted_text: String },

    /// Add a mark to a range.
    AddMark {
        range: Range,
        mark_type: String,
        attrs: HashMap<String, String>,
    },

    /// Remove a mark from a range.
    RemoveMark { range: Range, mark_type: String },

    /// Split a block at a position.
    SplitBlock {
        position: Position,
        /// Text that was moved to the new block
        moved_text: String,
    },

    /// Join two blocks (inverse of split).
    JoinBlocks {
        /// Position at the end of the first block (where they join)
        position: Position,
        /// Text from the second block that was joined
        second_block_text: String,
    },

    /// Set block type.
    SetBlockType {
        block_index: usize,
        old_type: String,
        new_type: String,
        old_attrs: HashMap<String, String>,
        new_attrs: HashMap<String, String>,
    },
}

impl UndoOperation {
    /// Get the inverse operation that undoes this operation.
    pub fn inverse(&self) -> Self {
        match self {
            UndoOperation::InsertText { position, text } => UndoOperation::DeleteText {
                range: Range::new(*position, *position + text.len()),
                deleted_text: text.clone(),
            },

            UndoOperation::DeleteText {
                range,
                deleted_text,
            } => UndoOperation::InsertText {
                position: range.start,
                text: deleted_text.clone(),
            },

            UndoOperation::AddMark {
                range,
                mark_type,
                attrs: _,
            } => UndoOperation::RemoveMark {
                range: *range,
                mark_type: mark_type.clone(),
            },

            UndoOperation::RemoveMark { range, mark_type } => UndoOperation::AddMark {
                range: *range,
                mark_type: mark_type.clone(),
                attrs: HashMap::new(), // Note: we lose attrs on undo of remove
            },

            UndoOperation::SplitBlock {
                position,
                moved_text,
            } => UndoOperation::JoinBlocks {
                position: *position,
                second_block_text: moved_text.clone(),
            },

            UndoOperation::JoinBlocks {
                position,
                second_block_text,
            } => UndoOperation::SplitBlock {
                position: *position,
                moved_text: second_block_text.clone(),
            },

            UndoOperation::SetBlockType {
                block_index,
                old_type,
                new_type,
                old_attrs,
                new_attrs,
            } => UndoOperation::SetBlockType {
                block_index: *block_index,
                old_type: new_type.clone(),
                new_type: old_type.clone(),
                old_attrs: new_attrs.clone(),
                new_attrs: old_attrs.clone(),
            },
        }
    }

    /// Check if this operation can be merged with another consecutive operation.
    /// Used for grouping consecutive typing into a single undo step.
    pub fn can_merge(&self, other: &UndoOperation) -> bool {
        match (self, other) {
            (
                UndoOperation::InsertText {
                    position: pos1,
                    text: text1,
                },
                UndoOperation::InsertText {
                    position: pos2,
                    text: text2,
                },
            ) => {
                // Can merge if the second insertion is right after the first
                // and both are single characters (typing)
                *pos2 == *pos1 + text1.len()
                    && text1.chars().count() == 1
                    && text2.chars().count() == 1
                    && !text1.contains('\n')
                    && !text2.contains('\n')
            }
            _ => false,
        }
    }

    /// Merge another operation into this one.
    /// Assumes can_merge() returned true.
    pub fn merge(&mut self, other: &UndoOperation) {
        match (self, other) {
            (
                UndoOperation::InsertText { text: text1, .. },
                UndoOperation::InsertText { text: text2, .. },
            ) => {
                text1.push_str(text2);
            }
            _ => {
                // Should not happen if can_merge was checked
                panic!("Cannot merge incompatible operations");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_inverse_is_delete() {
        let op = UndoOperation::InsertText {
            position: Position(5),
            text: "Hello".into(),
        };
        let inv = op.inverse();
        assert_eq!(
            inv,
            UndoOperation::DeleteText {
                range: Range::new(5usize, 10usize),
                deleted_text: "Hello".into(),
            }
        );
    }

    #[test]
    fn delete_inverse_is_insert() {
        let op = UndoOperation::DeleteText {
            range: Range::new(3usize, 8usize),
            deleted_text: "world".into(),
        };
        let inv = op.inverse();
        assert_eq!(
            inv,
            UndoOperation::InsertText {
                position: Position(3),
                text: "world".into(),
            }
        );
    }

    #[test]
    fn add_mark_inverse_is_remove_mark() {
        let op = UndoOperation::AddMark {
            range: Range::new(0usize, 5usize),
            mark_type: "bold".into(),
            attrs: HashMap::new(),
        };
        let inv = op.inverse();
        assert_eq!(
            inv,
            UndoOperation::RemoveMark {
                range: Range::new(0usize, 5usize),
                mark_type: "bold".into(),
            }
        );
    }

    #[test]
    fn remove_mark_inverse_is_add_mark() {
        let op = UndoOperation::RemoveMark {
            range: Range::new(0usize, 5usize),
            mark_type: "italic".into(),
        };
        let inv = op.inverse();
        assert_eq!(
            inv,
            UndoOperation::AddMark {
                range: Range::new(0usize, 5usize),
                mark_type: "italic".into(),
                attrs: HashMap::new(),
            }
        );
    }

    #[test]
    fn split_block_inverse_is_join_blocks() {
        let op = UndoOperation::SplitBlock {
            position: Position(5),
            moved_text: "World".into(),
        };
        let inv = op.inverse();
        assert_eq!(
            inv,
            UndoOperation::JoinBlocks {
                position: Position(5),
                second_block_text: "World".into(),
            }
        );
    }

    #[test]
    fn join_blocks_inverse_is_split_block() {
        let op = UndoOperation::JoinBlocks {
            position: Position(5),
            second_block_text: "Text".into(),
        };
        let inv = op.inverse();
        assert_eq!(
            inv,
            UndoOperation::SplitBlock {
                position: Position(5),
                moved_text: "Text".into(),
            }
        );
    }

    #[test]
    fn set_block_type_inverse_swaps_types() {
        let mut old_attrs = HashMap::new();
        old_attrs.insert("level".into(), "1".into());
        let mut new_attrs = HashMap::new();
        new_attrs.insert("level".into(), "2".into());

        let op = UndoOperation::SetBlockType {
            block_index: 0,
            old_type: "paragraph".into(),
            new_type: "heading".into(),
            old_attrs: old_attrs.clone(),
            new_attrs: new_attrs.clone(),
        };
        let inv = op.inverse();
        assert_eq!(
            inv,
            UndoOperation::SetBlockType {
                block_index: 0,
                old_type: "heading".into(),
                new_type: "paragraph".into(),
                old_attrs: new_attrs,
                new_attrs: old_attrs,
            }
        );
    }

    #[test]
    fn double_inverse_returns_original() {
        let op = UndoOperation::InsertText {
            position: Position(0),
            text: "test".into(),
        };
        let inv = op.inverse();
        let inv_inv = inv.inverse();

        // Should match original operation
        match (&op, &inv_inv) {
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
            _ => panic!("Double inverse did not return insert"),
        }
    }

    #[test]
    fn can_merge_consecutive_single_char_inserts() {
        let op1 = UndoOperation::InsertText {
            position: Position(0),
            text: "a".into(),
        };
        let op2 = UndoOperation::InsertText {
            position: Position(1),
            text: "b".into(),
        };
        assert!(op1.can_merge(&op2));
    }

    #[test]
    fn cannot_merge_non_consecutive_inserts() {
        let op1 = UndoOperation::InsertText {
            position: Position(0),
            text: "a".into(),
        };
        let op2 = UndoOperation::InsertText {
            position: Position(5),
            text: "b".into(),
        };
        assert!(!op1.can_merge(&op2));
    }

    #[test]
    fn cannot_merge_multi_char_inserts() {
        let op1 = UndoOperation::InsertText {
            position: Position(0),
            text: "ab".into(),
        };
        let op2 = UndoOperation::InsertText {
            position: Position(2),
            text: "c".into(),
        };
        assert!(!op1.can_merge(&op2));
    }

    #[test]
    fn cannot_merge_insert_with_newline() {
        let op1 = UndoOperation::InsertText {
            position: Position(0),
            text: "\n".into(),
        };
        let op2 = UndoOperation::InsertText {
            position: Position(1),
            text: "a".into(),
        };
        assert!(!op1.can_merge(&op2));
    }

    #[test]
    fn cannot_merge_different_operation_types() {
        let op1 = UndoOperation::InsertText {
            position: Position(0),
            text: "a".into(),
        };
        let op2 = UndoOperation::DeleteText {
            range: Range::new(0usize, 1usize),
            deleted_text: "a".into(),
        };
        assert!(!op1.can_merge(&op2));
    }

    #[test]
    fn merge_updates_text() {
        let mut op1 = UndoOperation::InsertText {
            position: Position(0),
            text: "a".into(),
        };
        let op2 = UndoOperation::InsertText {
            position: Position(1),
            text: "b".into(),
        };
        op1.merge(&op2);

        match op1 {
            UndoOperation::InsertText { position, text } => {
                assert_eq!(position, Position(0));
                assert_eq!(text, "ab");
            }
            _ => panic!("Expected InsertText"),
        }
    }

    #[test]
    fn merge_multiple_characters() {
        let mut op = UndoOperation::InsertText {
            position: Position(0),
            text: "h".into(),
        };
        for (i, ch) in "ello".chars().enumerate() {
            let next = UndoOperation::InsertText {
                position: Position(i + 1),
                text: ch.to_string(),
            };
            op.merge(&next);
        }

        match op {
            UndoOperation::InsertText { text, .. } => {
                assert_eq!(text, "hello");
            }
            _ => panic!("Expected InsertText"),
        }
    }
}
