use crate::{Invertible, Position, Range};

/// Operations for undo/redo in basic text editing.
#[derive(Clone, Debug, PartialEq)]
pub enum TextOperation {
    Insert { pos: Position, text: String },
    Delete { range: Range, deleted_text: String },
}

impl Invertible for TextOperation {
    fn inverse(&self) -> Self {
        match self {
            TextOperation::Insert { pos, text } => TextOperation::Delete {
                range: Range::new(*pos, Position(pos.0 + text.len())),
                deleted_text: text.clone(),
            },
            TextOperation::Delete {
                range,
                deleted_text,
            } => TextOperation::Insert {
                pos: range.start,
                text: deleted_text.clone(),
            },
        }
    }
}
