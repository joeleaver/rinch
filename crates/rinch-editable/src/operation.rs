use crate::{Invertible, Position, Range};

/// Operations for undo/redo in basic text editing.
#[derive(Clone, Debug, PartialEq)]
pub enum TextOperation {
    Insert {
        pos: Position,
        text: String,
    },
    Delete {
        range: Range,
        deleted_text: String,
    },
    /// Several operations that undo and redo as **one** step, applied in
    /// order (issue #288): a keystroke that replaced a selection (a delete and
    /// an insert), an adopted rewrite of the whole text, or a keystroke
    /// together with the rewrite its own `oninput` made in answer to it.
    Group(Vec<TextOperation>),
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
            // Undoing a sequence undoes its steps last-first.
            TextOperation::Group(ops) => {
                TextOperation::Group(ops.iter().rev().map(Invertible::inverse).collect())
            }
        }
    }
}
